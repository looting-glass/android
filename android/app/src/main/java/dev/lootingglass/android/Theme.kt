package dev.lootingglass.android

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Looting Glass's look (qml/KdeLook.qml in the desktop app): dark, purple accent. Same palette
 * values, so the phone app and the desktop overlay read as one product.
 */
object Lg {
    val window = Color(0xFF1A1A24)      // page / card background
    val windowText = Color(0xFFE2E2EC)
    val base = Color(0xFF13131A)        // input fields
    val alternateBase = Color(0xFF222230)
    val button = Color(0xFF262630)
    val accent = Color(0xFF8859A4)      // "highlight"
    val link = Color(0xFFB393D0)
    val placeholder = Color(0xFF707D8A)
    val mid = Color(0xFF2D2D3E)         // borders
    val midlight = Color(0xFF303043)
    val dark = Color(0xFF0D0D12)
    val red = Color(0xFFE06C6C)
    val green = Color(0xFF6CC070)

    val scheme = darkColorScheme(
        primary = accent, onPrimary = Color.White,
        primaryContainer = accent.copy(alpha = 0.3f), onPrimaryContainer = windowText,
        secondary = link, onSecondary = Color.White,
        background = window, onBackground = windowText,
        surface = window, onSurface = windowText,
        surfaceVariant = alternateBase, onSurfaceVariant = placeholder,
        surfaceContainer = button, surfaceContainerHigh = button, surfaceContainerHighest = alternateBase,
        outline = mid, outlineVariant = mid,
        error = red,
    )
}

/** Desktop AccentTint: a subtle accent tint with a solid accent bar on the left when active. */
fun Modifier.accentTint(active: Boolean): Modifier =
    if (active) background(Lg.accent.copy(alpha = 0.12f), RoundedCornerShape(4.dp)) else this

@Composable
fun AccentBar(active: Boolean) {
    if (active) Box(Modifier.padding(vertical = 4.dp).width(3.dp).fillMaxHeight().background(Lg.accent, RoundedCornerShape(1.5.dp)))
    else Box(Modifier.width(3.dp))
}

/** Desktop LgCheckBox off KDE: a field-coloured box with a soft outline and a tick. */
@Composable
fun LgCheck(checked: Boolean) {
    Box(
        Modifier.size(20.dp)
            .background(Lg.base, RoundedCornerShape(3.dp))
            .border(1.dp, Lg.windowText.copy(alpha = if (checked) 0.7f else 0.45f), RoundedCornerShape(3.dp)),
        contentAlignment = Alignment.Center,
    ) { if (checked) Text("✓", color = Lg.windowText, fontSize = 14.sp, fontWeight = FontWeight.Bold) }
}

/** Desktop Chip: small bold label in a thin outline; accent outline when highlighted. */
@Composable
fun LgBadge(text: String, accent: Boolean = false, warn: Boolean = false) {
    Box(
        Modifier.border(1.dp, if (warn) Lg.red else if (accent) Lg.accent else Lg.mid, RoundedCornerShape(4.dp))
            .padding(horizontal = 7.dp, vertical = 3.dp)
    ) { Text(text, fontSize = 11.sp, fontWeight = FontWeight.Bold, color = if (warn) Lg.red else Lg.windowText) }
}

@Composable
fun LgTheme(content: @Composable () -> Unit) = MaterialTheme(colorScheme = Lg.scheme, content = content)
