use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::time::{Duration, Instant};

const PICK_TIMEOUT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_millis(150);

fn with_activity<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let out = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        return Err("the Android call raised an exception".into());
    }
    out.map_err(|e| e.to_string())
}

fn optional_string(env: &mut JNIEnv, value: JObject) -> jni::errors::Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(&JString::from(value))?.into()))
}

fn wait_for_picked() -> Result<Vec<String>, String> {
    let started = Instant::now();
    while started.elapsed() < PICK_TIMEOUT {
        std::thread::sleep(POLL);
        let picked = with_activity(|env, a| {
            let v = env.call_method(a, "takePicked", "()Ljava/lang/String;", &[])?.l()?;
            optional_string(env, v)
        })?;
        if let Some(joined) = picked {
            return Ok(crate::documents::parse_picked(&joined));
        }
    }
    Err("the file picker timed out".into())
}

fn call_string(env: &mut JNIEnv, a: &JObject, method: &str, uri: &str) -> jni::errors::Result<Option<String>> {
    let juri = env.new_string(uri)?;
    let v = env.call_method(a, method, "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&juri)])?.l()?;
    optional_string(env, v)
}

pub struct Picked {
    pub name: String,
    pub mime: Option<String>,
    pub data: Result<Vec<u8>, String>,
}

pub fn open_documents() -> Result<Vec<Picked>, String> {
    with_activity(|env, a| env.call_method(a, "openDocuments", "()V", &[]).map(|_| ()))?;
    let mut out = Vec::new();
    for uri in wait_for_picked()? {
        let (name, mime, data) = with_activity(|env, a| {
            let name = call_string(env, a, "documentName", &uri)?;
            let mime = call_string(env, a, "documentMime", &uri)?;
            let juri = env.new_string(&uri)?;
            let bytes = env.call_method(a, "readDocument", "(Ljava/lang/String;)[B", &[JValue::Object(&juri)])?.l()?;
            let data = if bytes.is_null() { None } else { Some(env.convert_byte_array(JByteArray::from(bytes))?) };
            Ok((name, mime, data))
        })?;
        out.push(Picked {
            name: crate::documents::display_name(&uri, name.as_deref()),
            mime,
            data: data.ok_or_else(|| format!("Could not read {uri}")),
        });
    }
    Ok(out)
}

pub fn save_document(name: &str, mime: &str, data: &[u8]) -> Result<Option<String>, String> {
    with_activity(|env, a| {
        let jname = env.new_string(name)?;
        let jmime = env.new_string(mime)?;
        env.call_method(
            a,
            "createDocument",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&jname), JValue::Object(&jmime)],
        )
        .map(|_| ())
    })?;
    let Some(uri) = wait_for_picked()?.into_iter().next() else { return Ok(None) };
    let failure = with_activity(|env, a| {
        let juri = env.new_string(&uri)?;
        let bytes = env.byte_array_from_slice(data)?;
        let v = env
            .call_method(
                a,
                "writeDocument",
                "(Ljava/lang/String;[B)Ljava/lang/String;",
                &[JValue::Object(&juri), JValue::Object(&bytes)],
            )?
            .l()?;
        optional_string(env, v)
    })?;
    match failure {
        Some(e) => Err(e),
        None => Ok(Some(uri)),
    }
}

pub fn request_notification_permission() {
    let _ = with_activity(|env, a| env.call_method(a, "requestNotifications", "()V", &[]).map(|_| ()));
}

pub fn notifications_allowed() -> bool {
    with_activity(|env, a| env.call_method(a, "notificationsAllowed", "()Z", &[])?.z()).unwrap_or(false)
}

pub fn post_notification(account: usize, title: &str, body: &str, focus: u64) -> Result<(), String> {
    with_activity(|env, a| {
        let jtitle = env.new_string(title)?;
        let jbody = env.new_string(body)?;
        env.call_method(
            a,
            "postNotification",
            "(ILjava/lang/String;Ljava/lang/String;J)V",
            &[JValue::Int(account as i32), JValue::Object(&jtitle), JValue::Object(&jbody), JValue::Long(focus as i64)],
        )
        .map(|_| ())
    })
}

pub fn sync_service(on: bool) {
    let _ = with_activity(|env, a| env.call_method(a, "syncService", "(Z)V", &[JValue::Bool(on as u8)]).map(|_| ()));
}

pub fn take_focus() -> Option<u64> {
    let id = with_activity(|env, a| env.call_method(a, "takeFocus", "()J", &[])?.j()).ok()?;
    (id >= 0).then_some(id as u64)
}
