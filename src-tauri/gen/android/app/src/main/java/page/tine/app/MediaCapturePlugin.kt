package page.tine.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.graphics.BitmapFactory
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.os.Parcelable
import android.provider.MediaStore
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
import java.io.FileOutputStream

// Camera / photo-picker capture and voice-memo recording for the mobile editor
// toolbar (#7 camera, mic). Both return bounded native cache-file tokens; Rust
// streams those files into the graph so allowed media is never multiplied
// through base64 bridge round trips.
private const val TAG = "Tine/MediaCapture"
private const val MAX_PHOTO_BYTES = 64L * 1024L * 1024L
private const val MAX_PHOTO_PIXELS = 64L * 1024L * 1024L
private const val MAX_RECORDING_BYTES = 32L * 1024L * 1024L
private const val MAX_RECORDING_DURATION_MS = 30 * 60 * 1000

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
  private var recordingStoppedAtLimit = false

  // --- Photo: a single "take or choose" chooser (camera capture + file pick) ---

  @Command
  fun capturePhoto(invoke: Invoke) {
    try {
      // Keep at most one failed/recoverable photo token between attempts.
      activity.cacheDir.listFiles()
        ?.filter { it.name.startsWith("tine_photo_") }
        ?.forEach { it.delete() }
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

  private fun copyPickedPhoto(uri: Uri, out: File): Long {
    val input = activity.contentResolver.openInputStream(uri)
      ?: throw IllegalArgumentException("No image data returned")
    input.use { source ->
      FileOutputStream(out, false).use { target ->
        val buffer = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
          val count = source.read(buffer)
          if (count < 0) break
          if (count == 0) continue
          total += count
          if (total > MAX_PHOTO_BYTES) {
            throw IllegalArgumentException("Image exceeded the 64 MiB limit")
          }
          target.write(buffer, 0, count)
        }
        target.fd.sync()
        return total
      }
    }
  }

  private fun validatePhotoDimensions(photo: File) {
    val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(photo.absolutePath, options)
    val width = options.outWidth
    val height = options.outHeight
    if (width <= 0 || height <= 0) {
      throw IllegalArgumentException("Captured file is not a supported image")
    }
    if (width.toLong() * height.toLong() > MAX_PHOTO_PIXELS) {
      throw IllegalArgumentException("Image dimensions exceed the 64-megapixel limit")
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
          val bytes = if (uri != null && photo != null) {
            // Stream a picked image into our bounded native cache token and keep
            // its real extension for the final graph asset name.
            val mime = activity.contentResolver.getType(uri)
            MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)?.let { ext = it }
            copyPickedPhoto(uri, photo)
          } else if (photo != null && photo.exists() && photo.length() > 0) {
            // Camera wrote the full-res jpeg directly into our temp file.
            photo.length()
          } else {
            0L
          }
          if (bytes <= 0L) {
            photo?.delete()
            invoke.reject("No image data returned")
            return
          }
          if (bytes > MAX_PHOTO_BYTES) {
            photo?.delete()
            invoke.reject("Image exceeded the 64 MiB limit")
            return
          }
          validatePhotoDimensions(photo!!)
          val ret = JSObject()
          ret.put("status", "ok")
          ret.put("path", photo.absolutePath)
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
      // A process death or failed frontend import may leave one recoverable
      // memo in cache. Retire stale memos before starting another so repeated
      // failures cannot grow cache without bound.
      activity.cacheDir.listFiles()
        ?.filter { it.name.startsWith("tine_memo_") && it.name.endsWith(".m4a") }
        ?.forEach { it.delete() }
      val out = File.createTempFile("tine_memo_", ".m4a", activity.cacheDir)
      @Suppress("DEPRECATION")
      val rec = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        MediaRecorder(activity)
      } else {
        MediaRecorder()
      }
      // Own both resources before any fallible codec setup so prepare/start
      // failures release the recorder and delete the temp instead of leaking.
      recorder = rec
      recordFile = out
      recordingStoppedAtLimit = false
      rec.setAudioSource(MediaRecorder.AudioSource.MIC)
      rec.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
      rec.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
      rec.setAudioEncodingBitRate(128000)
      rec.setAudioSamplingRate(44100)
      rec.setMaxDuration(MAX_RECORDING_DURATION_MS)
      rec.setMaxFileSize(MAX_RECORDING_BYTES)
      rec.setOutputFile(out.absolutePath)
      rec.setOnInfoListener { stopped, what, _ ->
        if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED ||
          what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED) {
          try { stopped.stop() } catch (_: Exception) {}
          try { stopped.release() } catch (_: Exception) {}
          if (recorder === stopped) recorder = null
          recordingStoppedAtLimit = true
        }
      }
      rec.prepare()
      rec.start()
      val ret = JSObject()
      ret.put("status", "recording")
      invoke.resolve(ret)
    } catch (ex: Exception) {
      val out = recordFile
      releaseRecorder()
      out?.delete()
      invoke.reject(ex.message ?: "Failed to start recording")
    }
  }

  @Command
  fun stopRecording(invoke: Invoke) {
    val rec = recorder
    val out = recordFile
    if ((rec == null && !recordingStoppedAtLimit) || out == null) {
      invoke.reject("Not recording")
      return
    }
    if (rec != null) {
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
    }
    releaseRecorder()
    try {
      if (!out.exists() || out.length() <= 0) {
        out.delete()
        invoke.reject("No audio data recorded")
        return
      }
      if (out.length() > MAX_RECORDING_BYTES) {
        out.delete()
        invoke.reject("Recording exceeded the 32 MiB limit")
        return
      }
      val ret = JSObject()
      ret.put("status", "ok")
      ret.put("path", out.absolutePath)
      ret.put("ext", "m4a")
      invoke.resolve(ret)
    } catch (ex: Exception) {
      // Preserve the bounded native temp on import/finalization failure so the
      // recording is recoverable from app cache instead of being destroyed at
      // the failure boundary.
      invoke.reject(ex.message ?: "Failed to finalize recording")
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
    recordingStoppedAtLimit = false
  }
}
