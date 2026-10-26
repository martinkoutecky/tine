package page.tine.app

/** Host-internal opaque sRGB payload; invalid colours retain the mode fallback. */
internal object SystemBarColor {
  fun parse(value: String?): Int? {
    if (value == null || !Regex("#[0-9a-fA-F]{6}").matches(value)) return null
    return (0xff000000L or value.substring(1).toLong(16)).toInt()
  }

  fun usesLightIcons(color: Int): Boolean {
    fun linear(channel: Int): Double {
      val value = channel / 255.0
      return if (value <= 0.04045) value / 12.92 else Math.pow((value + 0.055) / 1.055, 2.4)
    }
    val luminance = 0.2126 * linear((color shr 16) and 255) +
      0.7152 * linear((color shr 8) and 255) + 0.0722 * linear(color and 255)
    // Choose whichever of white/black yields the greater contrast ratio.
    return luminance < (Math.sqrt(1.05 * 0.05) - 0.05)
  }
}
