package io.v0l.mailbench

import android.Manifest
import android.app.NativeActivity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns

class MainActivity : NativeActivity() {
    private val lock = Any()
    private var pending: String? = null
    private var focus: Long = -1L

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        remember(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        remember(intent)
    }

    private fun remember(intent: Intent?) {
        val id = intent?.getLongExtra(EXTRA_FOCUS, -1L) ?: -1L
        if (id >= 0) synchronized(lock) { focus = id }
    }

    fun takeFocus(): Long = synchronized(lock) {
        val id = focus
        focus = -1L
        id
    }

    fun openDocuments() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        synchronized(lock) { pending = null }
        runOnUiThread { startActivityForResult(intent, REQUEST_OPEN) }
    }

    fun createDocument(name: String, mime: String) {
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType(mime)
            .putExtra(Intent.EXTRA_TITLE, name)
        synchronized(lock) { pending = null }
        runOnUiThread { startActivityForResult(intent, REQUEST_CREATE) }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_OPEN && requestCode != REQUEST_CREATE) return
        val uris = ArrayList<String>()
        if (resultCode == RESULT_OK && data != null) {
            val clip = data.clipData
            if (clip != null) {
                for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri.toString())
            } else if (data.data != null) {
                uris.add(data.data.toString())
            }
        }
        synchronized(lock) { pending = uris.joinToString("\n") }
    }

    fun takePicked(): String? = synchronized(lock) {
        val value = pending
        pending = null
        value
    }

    fun documentName(uri: String): String? = try {
        contentResolver.query(Uri.parse(uri), arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    } catch (e: Exception) {
        null
    }

    fun documentMime(uri: String): String? = try {
        contentResolver.getType(Uri.parse(uri))
    } catch (e: Exception) {
        null
    }

    fun readDocument(uri: String): ByteArray? = try {
        contentResolver.openInputStream(Uri.parse(uri))?.use { it.readBytes() }
    } catch (e: Exception) {
        null
    }

    fun writeDocument(uri: String, data: ByteArray): String? = try {
        val out = contentResolver.openOutputStream(Uri.parse(uri), "wt")
        if (out == null) {
            "could not open the file"
        } else {
            out.use { it.write(data) }
            null
        }
    } catch (e: Exception) {
        e.message ?: e.toString()
    }

    fun requestNotifications() {
        if (Build.VERSION.SDK_INT < 33) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
        runOnUiThread { requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFY) }
    }

    fun notificationsAllowed(): Boolean {
        if (Build.VERSION.SDK_INT < 33) return true
        return checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
    }

    fun postNotification(account: Int, title: String, body: String, focus: Long) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(MAIL_CHANNEL, "New mail", NotificationManager.IMPORTANCE_DEFAULT))
        val open = Intent(this, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra(EXTRA_FOCUS, focus)
        val tap = PendingIntent.getActivity(this, account, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, MAIL_CHANNEL)
            .setSmallIcon(applicationInfo.icon)
            .setContentTitle(title)
            .setContentText(body.lineSequence().first())
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setContentIntent(tap)
            .setAutoCancel(true)
            .setGroup("mail-$account")
            .build()
        manager.notify(NOTIFICATION_BASE + account, notification)
    }

    fun syncService(on: Boolean) {
        val intent = Intent(this, SyncService::class.java)
        if (on) startForegroundService(intent) else stopService(intent)
    }

    companion object {
        const val EXTRA_FOCUS = "focus"
        const val MAIL_CHANNEL = "mail"
        const val NOTIFICATION_BASE = 100
        const val REQUEST_OPEN = 7101
        const val REQUEST_CREATE = 7102
        const val REQUEST_NOTIFY = 7103
    }
}
