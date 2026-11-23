package io.v0l.mailbench

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder

class SyncService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(SYNC_CHANNEL, "Mail sync", NotificationManager.IMPORTANCE_MIN))
        val open = Intent(this, MainActivity::class.java)
        val tap = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, SYNC_CHANNEL)
            .setSmallIcon(applicationInfo.icon)
            .setContentTitle("Mailbench")
            .setContentText("Checking for new mail")
            .setContentIntent(tap)
            .setOngoing(true)
            .build()
        startForeground(SYNC_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        return START_STICKY
    }

    companion object {
        const val SYNC_CHANNEL = "sync"
        const val SYNC_ID = 1
    }
}
