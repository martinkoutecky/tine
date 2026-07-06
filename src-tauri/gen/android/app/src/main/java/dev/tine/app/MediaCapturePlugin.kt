package dev.tine.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.os.Parcelable
import android.provider.MediaStore
import android.util.Base64
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

// Camera / photo-picker capture and voice-memo recording for the mobile editor
// toolbar (#7 camera, mic). Each command returns the captured bytes as base64
// in `data` with a file `ext`; the frontend writes them into the graph's
// `assets/` (backend save_asset) and inserts the media ref. Mirrors the
// GraphFolderPickerPlugin bridge pattern.
private const val TAG = "Tine/MediaCapture"

@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")
  ]
)
class MediaCapturePlugin(private val activity: Activity) : Plugin(activity) {
  // Temp file the camera app writes the full-resolution photo into (EXTRA_OUTPUT).
  private var pendingPhotoFile: File? = null
  // Active voice-memo recorder + its output file (null when not recording).
  private var recorder: MediaRecorder? = null
  private var recordFile: File? = null

  // --- Photo: a single "take or choose" chooser (camera capture + file pick) ---

  @Command
  fun capturePhoto(invoke: Invoke) {
    try {
      val photo = File.createTempFile("tine_photo_", ".jpg", activity.cacheDir)
      pendingPhotoFile = photo
      val outUri: Uri = FileProvider.getUriForFile(
        activity, "${activity.packageName}.fileprovider", photo
      )
      val captureIntent = Intent(MediaStore.ACTION_IMAGE_CAPTURE).apply {
        putExtra(MediaStore.EXTRA_OUTPUT, outUri)
        addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      // Base intent = pick an existing image; camera capture rides as an extra so
      // the system chooser offers both "take photo" and "choose from library".
      val pickIntent = Intent(Intent.ACTION_GET_CONTENT).apply {
        type = "image/*"
        addCategory(Intent.CATEGORY_OPENABLE)
      }
      val chooser = Intent.createChooser(pickIntent, "Add image").apply {
        putExtra(Intent.EXTRA_INITIAL_INTENTS, arrayOf<Parcelable>(captureIntent))
      }
      startActivityForResult(invoke, chooser, "photoResult")
    } catch (ex: Exception) {
      pendingPhotoFile?.delete()
      pendingPhotoFile = null
      invoke.reject(ex.message ?: "Failed to start image capture")
    }
  }

  @ActivityCallback
  fun photoResult(invoke: Invoke, result: ActivityResult) {
    val photo = pendingPhotoFile
    pendingPhotoFile = null
    try {
      when (result.resultCode) {
        Activity.RESULT_OK -> {
          val uri = result.data?.data
          var ext = "jpg"
          val bytes: ByteArray? = if (uri != null) {
            // Picked an existing file — read it and keep its real extension.
            val mime = activity.contentResolver.getType(uri)
            MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)?.let { ext = it }
            activity.contentResolver.openInputStream(uri)?.use { it.readBytes() }
          } else if (photo != null && photo.exists() && photo.length() > 0) {
            // Camera wrote the full-res jpeg into our temp file.
            photo.readBytes()
          } else {
            null
          }
          photo?.delete()
          if (bytes == null || bytes.isEmpty()) {
            invoke.reject("No image data returned")
            return
          }
          val ret = JSObject()
          ret.put("status", "ok")
          ret.put("data", Base64.encodeToString(bytes, Base64.NO_WRAP))
          ret.put("ext", ext)
          invoke.resolve(ret)
        }
        Activity.RESULT_CANCELED -> {
          photo?.delete()
          val ret = JSObject()
          ret.put("status", "cancelled")
          invoke.resolve(ret)
        }
        else -> {
          photo?.delete()
          invoke.reject("Image capture failed")
        }
      }
    } catch (ex: Exception) {
      photo?.delete()
      invoke.reject(ex.message ?: "Failed to read captured image")
    }
  }

  // --- Voice memo: start (permission-gated) / stop → bytes ---

  @Command
  fun startRecording(invoke: Invoke) {
    if (getPermissionState("microphone") != PermissionState.GRANTED) {
      requestPermissionForAlias("microphone", invoke, "recordPermissionResult")
      return
    }
    beginRecording(invoke)
  }

  @PermissionCallback
  fun recordPermissionResult(invoke: Invoke) {
    if (getPermissionState("microphone") == PermissionState.GRANTED) {
      beginRecording(invoke)
    } else {
      invoke.reject("Microphone permission denied")
    }
  }

  private fun beginRecording(invoke: Invoke) {
    if (recorder != null) {
      invoke.reject("Already recording")
      return
    }
    try {
      val out = File.createTempFile("tine_memo_", ".m4a", activity.cacheDir)
      @Suppress("DEPRECATION")
      val rec = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        MediaRecorder(activity)
      } else {
        MediaRecorder()
      }
      rec.setAudioSource(MediaRecorder.AudioSource.MIC)
      rec.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
      rec.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
      rec.setAudioEncodingBitRate(128000)
      rec.setAudioSamplingRate(44100)
      rec.setOutputFile(out.absolutePath)
      rec.prepare()
      rec.start()
      recorder = rec
      recordFile = out
      val ret = JSObject()
      ret.put("status", "recording")
      invoke.resolve(ret)
    } catch (ex: Exception) {
      releaseRecorder()
      invoke.reject(ex.message ?: "Failed to start recording")
    }
  }

  @Command
  fun stopRecording(invoke: Invoke) {
    val rec = recorder
    val out = recordFile
    if (rec == null || out == null) {
      invoke.reject("Not recording")
      return
    }
    try {
      rec.stop()
    } catch (ex: Exception) {
      // A stop() right after start() (empty recording) throws; treat as cancelled.
      Log.w(TAG, "MediaRecorder.stop failed: ${ex.message}")
      releaseRecorder()
      out.delete()
      val ret = JSObject()
      ret.put("status", "cancelled")
      invoke.resolve(ret)
      return
    }
    releaseRecorder()
    try {
      val bytes = if (out.exists() && out.length() > 0) out.readBytes() else null
      out.delete()
      if (bytes == null || bytes.isEmpty()) {
        invoke.reject("No audio data recorded")
        return
      }
      val ret = JSObject()
      ret.put("status", "ok")
      ret.put("data", Base64.encodeToString(bytes, Base64.NO_WRAP))
      ret.put("ext", "m4a")
      invoke.resolve(ret)
    } catch (ex: Exception) {
      out.delete()
      invoke.reject(ex.message ?: "Failed to read recording")
    }
  }

  @Command
  fun cancelRecording(invoke: Invoke) {
    val rec = recorder
    val out = recordFile
    if (rec != null) {
      try { rec.stop() } catch (_: Exception) {}
    }
    releaseRecorder()
    out?.delete()
    val ret = JSObject()
    ret.put("status", "cancelled")
    invoke.resolve(ret)
  }

  private fun releaseRecorder() {
    try { recorder?.release() } catch (_: Exception) {}
    recorder = null
    recordFile = null
  }
}
