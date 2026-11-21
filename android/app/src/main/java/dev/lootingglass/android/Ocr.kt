package dev.lootingglass.android

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.graphics.Rect
import android.net.Uri
import androidx.exifinterface.media.ExifInterface
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import uniffi.lootlens_ffi.OcrLine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** On-device OCR (ML Kit, bundled Latin model) plus text-colour sampling for rarity/mod kinds. */
object Ocr {
    private val recognizer by lazy { TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS) }

    /** Small screenshots are upscaled first: ML Kit wants letters well over 16 px tall. */
    private const val MIN_OCR_WIDTH = 1200

    suspend fun recognize(bitmap: Bitmap): List<OcrLine> {
        val scale = when {
            bitmap.width >= MIN_OCR_WIDTH -> 1f
            else -> min(3f, MIN_OCR_WIDTH.toFloat() / bitmap.width)
        }
        val input = if (scale == 1f) bitmap else withContext(Dispatchers.Default) {
            Bitmap.createScaledBitmap(bitmap, (bitmap.width * scale).roundToInt(), (bitmap.height * scale).roundToInt(), true)
        }
        val text = suspendCancellableCoroutine { cont ->
            recognizer.process(InputImage.fromBitmap(input, 0))
                .addOnSuccessListener { cont.resume(it) }
                .addOnFailureListener { cont.resumeWithException(it) }
        }
        return withContext(Dispatchers.Default) {
            text.textBlocks.flatMap { it.lines }.mapNotNull { line ->
                val b = line.boundingBox ?: return@mapNotNull null
                val r = Rect((b.left / scale).toInt(), (b.top / scale).toInt(), (b.right / scale).toInt(), (b.bottom / scale).toInt())
                OcrLine(
                    text = line.text,
                    left = r.left.toFloat(),
                    top = r.top.toFloat(),
                    right = r.right.toFloat(),
                    bottom = r.bottom.toFloat(),
                    color = sampleColor(bitmap, r),
                )
            }
        }
    }

    /**
     * Average colour of the brightest quarter of pixels in the box: the text, not the dark
     * tooltip background. Same method as the desktop eval harness. Returns 0xRRGGBB.
     */
    fun sampleColor(bmp: Bitmap, r: Rect): UInt? {
        val x0 = max(0, r.left); val y0 = max(0, r.top)
        val x1 = min(bmp.width, r.right); val y1 = min(bmp.height, r.bottom)
        if (x1 <= x0 || y1 <= y0) return null
        val w = x1 - x0; val h = y1 - y0
        val px = IntArray(w * h)
        bmp.getPixels(px, 0, w, x0, y0, w, h)
        val step = if (px.size > 40_000) 3 else 1
        val sample = px.filterIndexed { i, _ -> i % step == 0 }
        val lum = { p: Int -> 30 * ((p shr 16) and 0xff) + 59 * ((p shr 8) and 0xff) + 11 * (p and 0xff) }
        val top = sample.sortedByDescending(lum).take(max(1, sample.size / 4))
        var rs = 0L; var gs = 0L; var bs = 0L
        for (p in top) { rs += (p shr 16) and 0xff; gs += (p shr 8) and 0xff; bs += p and 0xff }
        val n = top.size
        return (((rs / n) shl 16) or ((gs / n) shl 8) or (bs / n)).toUInt()
    }

    /** OCR lines as JSON, the format the desktop eval harness reads (`<name>.ocr.json`). */
    fun toJson(lines: List<OcrLine>): String {
        val arr = JSONArray()
        for (l in lines) {
            val o = JSONObject()
            o.put("text", l.text); o.put("left", l.left.toDouble()); o.put("top", l.top.toDouble())
            o.put("right", l.right.toDouble()); o.put("bottom", l.bottom.toDouble())
            l.color?.let { c ->
                val v = c.toInt()
                o.put("color", JSONArray(listOf((v shr 16) and 0xff, (v shr 8) and 0xff, v and 0xff)))
            }
            arr.put(o)
        }
        return JSONObject().put("engine", "mlkit-text-recognition-16").put("lines", arr).toString(1)
    }

    /** Decode an image, applying EXIF rotation and capping the size (phone photos are huge). */
    fun load(context: Context, uri: Uri, maxSide: Int = 3000): Bitmap {
        val opts = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it, null, opts) }
        var sample = 1
        while (max(opts.outWidth, opts.outHeight) / sample > maxSide) sample *= 2
        val bmp = context.contentResolver.openInputStream(uri).use {
            BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sample })
        } ?: error("Could not read the image")
        val rotation = context.contentResolver.openInputStream(uri).use {
            it?.let { s -> ExifInterface(s).rotationDegrees } ?: 0
        }
        return rotate(bmp, rotation)
    }

    fun rotate(bmp: Bitmap, degrees: Int): Bitmap {
        if (degrees == 0) return bmp
        val m = Matrix().apply { postRotate(degrees.toFloat()) }
        return Bitmap.createBitmap(bmp, 0, 0, bmp.width, bmp.height, m, true)
    }
}
