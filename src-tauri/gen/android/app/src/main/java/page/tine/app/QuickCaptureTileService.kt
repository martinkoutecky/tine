package page.tine.app

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.service.quicksettings.TileService

/**
 * Quick Settings tile "Quick capture" (A3 of the native-integrations batch):
 * opens Tine on the `tine://capture` route, i.e. today's journal with an empty
 * bottom block being edited (src/deepLinkNavigation.ts). It writes nothing.
 */
class QuickCaptureTileService : TileService() {
  @SuppressLint("StartActivityAndCollapseDeprecated")
  override fun onClick() {
    super.onClick()
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse("tine://capture"), this, MainActivity::class.java)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val open = {
      if (Build.VERSION.SDK_INT >= 34) {
        startActivityAndCollapse(
          PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
        )
      } else {
        @Suppress("DEPRECATION")
        startActivityAndCollapse(intent)
      }
    }
    // The device stays locked until the user unlocks: Tine is not a lock-screen app.
    if (isLocked) unlockAndRun(open) else open()
  }
}
