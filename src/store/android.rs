use jni::objects::{JByteArray, JObject, JValue};
use jni::{JNIEnv, JavaVM};

use super::vault::{FileKeychain, Sealer};
use super::{Result, StoreError};
use std::path::PathBuf;
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.get().cloned()
}

pub fn keychain() -> Result<FileKeychain<KeystoreSealer>> {
    let dir = DATA_DIR.get().ok_or_else(|| StoreError::Keychain("data dir not set".into()))?;
    Ok(FileKeychain::new(dir.join("keychain"), KeystoreSealer))
}

const PROVIDER: &str = "AndroidKeyStore";
const ALIAS: &str = "mailbench-keychain";
const IV_LEN: usize = 12;
const ENCRYPT: i32 = 1;
const DECRYPT: i32 = 2;
const PURPOSES: i32 = 3;

pub struct KeystoreSealer;

fn err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Keychain(e.to_string())
}

fn string_array<'a>(env: &mut JNIEnv<'a>, value: &str) -> jni::errors::Result<JObject<'a>> {
    let class = env.find_class("java/lang/String")?;
    let first = env.new_string(value)?;
    let array = env.new_object_array(1, class, &first)?;
    Ok(JObject::from(array))
}

fn secret_key<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JObject<'a>> {
    let provider = env.new_string(PROVIDER)?;
    let alias = env.new_string(ALIAS)?;
    let store = env
        .call_static_method(
            "java/security/KeyStore",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(&store, "load", "(Ljava/security/KeyStore$LoadStoreParameter;)V", &[JValue::Object(&JObject::null())])?;
    let present = env
        .call_method(&store, "containsAlias", "(Ljava/lang/String;)Z", &[JValue::Object(&alias)])?
        .z()?;
    if !present {
        let aes = env.new_string("AES")?;
        let generator = env
            .call_static_method(
                "javax/crypto/KeyGenerator",
                "getInstance",
                "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
                &[JValue::Object(&aes), JValue::Object(&provider)],
            )?
            .l()?;
        let builder = env.new_object(
            "android/security/keystore/KeyGenParameterSpec$Builder",
            "(Ljava/lang/String;I)V",
            &[JValue::Object(&alias), JValue::Int(PURPOSES)],
        )?;
        let blocks = string_array(env, "GCM")?;
        let paddings = string_array(env, "NoPadding")?;
        let builder_sig = "Landroid/security/keystore/KeyGenParameterSpec$Builder;";
        env.call_method(&builder, "setBlockModes", format!("([Ljava/lang/String;){builder_sig}"), &[JValue::Object(&blocks)])?;
        env.call_method(&builder, "setEncryptionPaddings", format!("([Ljava/lang/String;){builder_sig}"), &[JValue::Object(&paddings)])?;
        env.call_method(&builder, "setKeySize", format!("(I){builder_sig}"), &[JValue::Int(256)])?;
        let spec = env.call_method(&builder, "build", "()Landroid/security/keystore/KeyGenParameterSpec;", &[])?.l()?;
        env.call_method(&generator, "init", "(Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Object(&spec)])?;
        env.call_method(&generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?;
    }
    env.call_method(
        &store,
        "getKey",
        "(Ljava/lang/String;[C)Ljava/security/Key;",
        &[JValue::Object(&alias), JValue::Object(&JObject::null())],
    )?
    .l()
}

fn cipher<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JObject<'a>> {
    let transform = env.new_string("AES/GCM/NoPadding")?;
    env.call_static_method(
        "javax/crypto/Cipher",
        "getInstance",
        "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
        &[JValue::Object(&transform)],
    )?
    .l()
}

fn do_final<'a>(env: &mut JNIEnv<'a>, cipher: &JObject<'a>, input: &[u8]) -> jni::errors::Result<Vec<u8>> {
    let input = env.byte_array_from_slice(input)?;
    let out = env.call_method(cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?.l()?;
    env.convert_byte_array(JByteArray::from(out))
}

fn seal(env: &mut JNIEnv, plain: &[u8]) -> jni::errors::Result<Vec<u8>> {
    let key = secret_key(env)?;
    let cipher = cipher(env)?;
    env.call_method(&cipher, "init", "(ILjava/security/Key;)V", &[JValue::Int(ENCRYPT), JValue::Object(&key)])?;
    let iv = env.call_method(&cipher, "getIV", "()[B", &[])?.l()?;
    let iv = env.convert_byte_array(JByteArray::from(iv))?;
    let body = do_final(env, &cipher, plain)?;
    let mut sealed = iv;
    sealed.extend(body);
    Ok(sealed)
}

fn open(env: &mut JNIEnv, sealed: &[u8]) -> jni::errors::Result<Vec<u8>> {
    let (iv, body) = sealed.split_at(IV_LEN);
    let key = secret_key(env)?;
    let cipher = cipher(env)?;
    let iv = env.byte_array_from_slice(iv)?;
    let spec = env.new_object("javax/crypto/spec/GCMParameterSpec", "(I[B)V", &[JValue::Int(128), JValue::Object(&iv)])?;
    env.call_method(
        &cipher,
        "init",
        "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[JValue::Int(DECRYPT), JValue::Object(&key), JValue::Object(&spec)],
    )?;
    do_final(env, &cipher, body)
}

fn with_env<T>(f: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>) -> Result<T> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(err)?;
    let mut env = vm.attach_current_thread().map_err(err)?;
    let out = f(&mut env);
    if out.is_err() {
        let _ = env.exception_clear();
    }
    out.map_err(err)
}

impl Sealer for KeystoreSealer {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>> {
        with_env(|env| seal(env, plain))
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() <= IV_LEN {
            return Err(StoreError::Keychain("sealed secret is too short".into()));
        }
        with_env(|env| open(env, sealed))
    }
}
