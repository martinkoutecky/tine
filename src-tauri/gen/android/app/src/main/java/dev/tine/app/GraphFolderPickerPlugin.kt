package dev.tine.app

import android.app.Activity
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.Settings
import android.util.Log
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@TauriPlugin
class GraphFolderPickerPlugin(private val activity: Activity): Plugin(activity) {
  @Command
  fun pickGraphFolder(invoke: Invoke) {
    if (!fileAccessAllowed()) {
      openAllFilesAccessSettings()
      val ret = JSObject()
      ret.put("status", "permission-requested")
      invoke.resolve(ret)
      return
    }

    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
    intent.addCategory(Intent.CATEGORY_DEFAULT)
    startActivityForResult(invoke, intent, "folderPickerResult")
  }

  @ActivityCallback
  fun folderPickerResult(invoke: Invoke, result: ActivityResult) {
    try {
      when (result.resultCode) {
        Activity.RESULT_OK -> {
          val treeUri = result.data?.data
          if (treeUri == null) {
            invoke.reject("Folder picker returned no URI")
            return
          }
          val docUri = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri)
          )
          val path = RealPathResolver.getPath(activity, docUri)
          Log.i("Tine/FolderPicker", "Resolved $docUri to $path")
          if (path.isNullOrEmpty()) {
            invoke.reject("Cannot resolve folder to a filesystem path: $docUri")
            return
          }
          val ret = JSObject()
          ret.put("status", "picked")
          ret.put("path", path)
          invoke.resolve(ret)
        }
        Activity.RESULT_CANCELED -> {
          val ret = JSObject()
          ret.put("status", "cancelled")
          invoke.resolve(ret)
        }
        else -> invoke.reject("Folder picker failed")
      }
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "Failed to read folder pick result")
    }
  }

  private fun fileAccessAllowed(): Boolean {
    return Build.VERSION.SDK_INT < Build.VERSION_CODES.R || Environment.isExternalStorageManager()
  }

  private fun openAllFilesAccessSettings() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
    try {
      val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
      intent.data = Uri.fromParts("package", activity.packageName, null)
      activity.startActivity(intent)
    } catch (ex: Exception) {
      val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
      activity.startActivity(intent)
    }
  }
}

private object RealPathResolver {
  fun getPath(context: Context, uri: Uri): String? {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.KITKAT && DocumentsContract.isDocumentUri(context, uri)) {
      if (isExternalStorageDocument(uri)) {
        val docId = DocumentsContract.getDocumentId(uri)
        val split = docId.split(":", limit = 2)
        val type = split[0]
        val remain = if (split.size == 2) split[1] else ""

        if ("primary".equals(type, ignoreCase = true)) {
          return Environment.getExternalStorageDirectory().toString() + "/" + remain
        } else if ("home".equals(type, ignoreCase = true)) {
          return Environment.getExternalStorageDirectory().toString() + "/Documents/" + remain
        } else if ("downloads".equals(type, ignoreCase = true)) {
          return Environment.getExternalStorageDirectory().toString() + "/Download/" + remain
        }

        for (mediaDir in context.externalMediaDirs) {
          val extPath = mediaDir.absolutePath
          if (extPath.contains("/$type/")) {
            val dir = File(extPath.substring(0, extPath.indexOf("/Android")) + "/" + remain)
            if (dir.exists()) return dir.absolutePath
          }
        }

        listOf(
          File("/storage/$type/$remain"),
          File("/mnt/media_rw/$type/$remain"),
          File("/mnt/$type/$remain")
        ).firstOrNull { it.exists() }?.let { return it.absolutePath }
      } else if (isDownloadsDocument(uri)) {
        val id = DocumentsContract.getDocumentId(uri)
        if (id.isNotEmpty()) {
          if (id.startsWith("raw:")) return id.removePrefix("raw:")
          return try {
            val contentUri = ContentUris.withAppendedId(
              Uri.parse("content://downloads/public_downloads"),
              id.toLong()
            )
            getDataColumn(context, contentUri, null, null)
          } catch (ex: NumberFormatException) {
            null
          }
        }
      } else if (isMediaDocument(uri)) {
        val docId = DocumentsContract.getDocumentId(uri)
        val split = docId.split(":", limit = 2)
        val type = split[0]
        if (split.size < 2) return null
        val contentUri = when (type) {
          "image" -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
          "video" -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
          "audio" -> MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
          else -> null
        }
        return getDataColumn(context, contentUri, "_id=?", arrayOf(split[1]))
      } else if (isTermuxDocument(uri)) {
        var docId = DocumentsContract.getDocumentId(uri)
        if (docId.startsWith("/")) {
          if (docId.contains("/com.termux/files/home/storage/")) {
            val remain = docId.replaceFirst(
              Regex("^.*?com\\.termux/files/home/storage/[^/]+/"),
              ""
            )
            docId = when {
              docId.contains("/storage/external-1") -> {
                val dirs = context.getExternalFilesDirs(remain)
                if (dirs != null && dirs.size >= 2 && dirs[1] != null) dirs[1]!!.absolutePath else docId
              }
              docId.contains("/storage/media-1") -> {
                val dirs = context.externalMediaDirs
                if (dirs.size >= 2) dirs[1].absolutePath + "/" + remain else docId
              }
              docId.contains("/storage/downloads") ->
                Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).toString() + "/" + remain
              docId.contains("/storage/shared") ->
                Environment.getExternalStorageDirectory().toString() + "/" + remain
              else -> docId
            }
          }
          val dir = File(docId)
          if (dir.exists()) return dir.absolutePath
          Log.e("Tine/FileUtil", "Handle termux content URL failed: $docId")
        }
      }
    } else if ("content".equals(uri.scheme, ignoreCase = true)) {
      if (isGooglePhotosUri(uri)) return uri.lastPathSegment
      return getDataColumn(context, uri, null, null)
    } else if ("file".equals(uri.scheme, ignoreCase = true)) {
      return uri.path
    }

    return null
  }

  private fun getDataColumn(
    context: Context,
    uri: Uri?,
    selection: String?,
    selectionArgs: Array<String>?
  ): String? {
    if (uri == null) return null
    var cursor: Cursor? = null
    val column = "_data"
    val projection = arrayOf(column)

    try {
      cursor = context.contentResolver.query(uri, projection, selection, selectionArgs, null)
      if (cursor != null && cursor.moveToFirst()) {
        val index = cursor.getColumnIndexOrThrow(column)
        return cursor.getString(index)
      }
    } finally {
      cursor?.close()
    }
    return null
  }

  private fun isExternalStorageDocument(uri: Uri): Boolean {
    return "com.android.externalstorage.documents" == uri.authority
  }

  private fun isDownloadsDocument(uri: Uri): Boolean {
    return "com.android.providers.downloads.documents" == uri.authority
  }

  private fun isMediaDocument(uri: Uri): Boolean {
    return "com.android.providers.media.documents" == uri.authority
  }

  private fun isGooglePhotosUri(uri: Uri): Boolean {
    return "com.google.android.apps.photos.content" == uri.authority
  }

  private fun isTermuxDocument(uri: Uri): Boolean {
    return "com.termux.documents" == uri.authority
  }
}
