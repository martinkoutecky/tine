package page.tine.app

import org.junit.Assert.*
import org.junit.Test

class SystemBarColorTest {
  @Test fun opaquePayloadAndFallback() {
    assertEquals(0xff002b36.toInt(), SystemBarColor.parse("#002b36"))
    assertEquals(0xfffaf0dc.toInt(), SystemBarColor.parse("#FAF0DC"))
    for (value in listOf(null, "transparent", "", "#00000000", "red", "#xyzxyz")) {
      assertNull(SystemBarColor.parse(value))
    }
  }

  @Test fun iconContrastFollowsColorRegardlessOfMode() {
    assertTrue(SystemBarColor.usesLightIcons(SystemBarColor.parse("#002b36")!!))
    assertFalse(SystemBarColor.usesLightIcons(SystemBarColor.parse("#faf0dc")!!))
    assertTrue(SystemBarColor.usesLightIcons(0xff000000.toInt()))
    assertFalse(SystemBarColor.usesLightIcons(0xffffffff.toInt()))
  }
}
