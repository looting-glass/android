package dev.lootingglass.android

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.ZoomState
import androidx.camera.view.CameraController
import androidx.camera.view.LifecycleCameraController
import androidx.camera.view.PreviewView
import androidx.lifecycle.Observer
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import uniffi.lootlens_ffi.GameId
import uniffi.lootlens_ffi.Parsed
import uniffi.lootlens_ffi.RealmId

class MainActivity : ComponentActivity() {
    private val vm: LensViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handleIntent(intent)
        setContent {
            LgTheme {
                Surface(Modifier.fillMaxSize(), color = Lg.window) { App(vm) }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent) {
        if (intent.getBooleanExtra("batch", false)) {
            vm.runBatch()
            return
        }
        if (intent.action == Intent.ACTION_SEND) {
            val uri = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            else @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_STREAM)
            uri?.let { runCatching { vm.check(Ocr.load(this, it)) } }
        }
    }
}

private val Dim = Lg.windowText.copy(alpha = 0.6f)

@Composable
fun App(vm: LensViewModel) {
    val s by vm.state.collectAsStateWithLifecycle()
    var camera by remember { mutableStateOf(false) }
    val ctx = LocalContext.current

    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri?.let { runCatching { vm.check(Ocr.load(ctx, it)) }.onFailure { e -> android.widget.Toast.makeText(ctx, e.message, android.widget.Toast.LENGTH_LONG).show() } }
    }
    val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok -> if (ok) camera = true }

    if (camera) {
        BackHandler { camera = false }
        CameraScreen(onCaptured = { camera = false; vm.check(it) }, onClose = { camera = false })
        return
    }

    LazyColumn(
        Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Image(androidx.compose.ui.res.painterResource(R.drawable.ic_launcher_foreground), null, Modifier.size(36.dp).padding(end = 2.dp))
                Text("LOOTING GLASS", fontSize = 16.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.sp, color = Lg.windowText)
            }
        }
        item { SettingsRow(s, vm) }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) camera = true
                    else askCamera.launch(Manifest.permission.CAMERA)
                }, enabled = s.busy == null, shape = RoundedCornerShape(4.dp), modifier = Modifier.weight(1f).height(48.dp)) { Text("Take photo") }
                OutlinedButton(onClick = { pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                    enabled = s.busy == null, shape = RoundedCornerShape(4.dp), border = BorderStroke(1.dp, Lg.midlight),
                    colors = ButtonDefaults.outlinedButtonColors(containerColor = Lg.button, contentColor = Lg.windowText),
                    modifier = Modifier.weight(1f).height(48.dp)) { Text("Pick image") }
            }
        }
        if (s.dataStatus.isNotEmpty()) item { Text(s.dataStatus, fontSize = 12.sp, color = Lg.placeholder) }
        s.busy?.let { item { Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp); Spacer(Modifier.width(8.dp)); Text(it) } } }
        s.error?.let { item { Text(it, color = Lg.red) } }
        s.image?.let { bmp ->
            item {
                Image(bmp.asImageBitmap(), null, Modifier.fillMaxWidth().heightIn(max = 220.dp), contentScale = ContentScale.Fit)
            }
        }
        s.parsed?.let { p -> parsedItems(p, s, vm) }
        s.price?.let { pr ->
            item {
                Row(Modifier.padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Matched: ${pr.total}", modifier = Modifier.padding(end = 8.dp))
                    if (s.searching) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    Text(LensViewModel.realmLabel(s), fontSize = 12.sp, color = Dim, modifier = Modifier.weight(1f).padding(start = 8.dp))
                    OutlinedButton(onClick = { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(pr.url))) },
                        shape = RoundedCornerShape(4.dp), border = BorderStroke(1.dp, Lg.midlight),
                        colors = ButtonDefaults.outlinedButtonColors(containerColor = Lg.button, contentColor = Lg.windowText),
                        contentPadding = PaddingValues(start = 12.dp, end = 10.dp)) { Text("Trade ↗") }
                }
            }
            if (pr.listings.isNotEmpty()) item {
                Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp)) {
                    Text("Price", Modifier.weight(1.4f), fontWeight = FontWeight.Bold)
                    Text("iLvl", Modifier.weight(0.6f), fontWeight = FontWeight.Bold)
                    Text("Listed", Modifier.weight(0.8f), fontWeight = FontWeight.Bold)
                    Text("Seller", Modifier.weight(1.4f), fontWeight = FontWeight.Bold)
                }
            }
            itemsIndexed(pr.listings) { i, l ->
                // Zebra stripes from the text colour, as on the desktop.
                Row(Modifier.fillMaxWidth().background(if (i % 2 == 1) Lg.windowText.copy(alpha = 0.05f) else Color.Transparent)
                    .padding(horizontal = 4.dp, vertical = 5.dp)) {
                    Text(l.price, Modifier.weight(1.4f), fontWeight = FontWeight.Bold, maxLines = 1)
                    Text(l.ilvl?.takeIf { it > 0u }?.toString() ?: "—", Modifier.weight(0.6f))
                    Text(l.age, Modifier.weight(0.8f), color = Lg.windowText.copy(alpha = 0.8f))
                    Text(l.seller, Modifier.weight(1.4f), maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                        color = if (l.corrupted) Lg.red else Dim)
                }
            }
            if (pr.listings.isEmpty()) item { Text("No listings found — try disabling some filters, or turn off \"Online only\".", color = Lg.windowText.copy(alpha = 0.7f)) }
        }
        item {
            Spacer(Modifier.height(16.dp))
            val version = remember { runCatching { ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName }.getOrNull() ?: "?" }
            Text("Looting Glass $version", fontSize = 12.sp, color = Dim)
            Text("Proof of concept. Fan-made; not affiliated with or endorsed by Grinding Gear Games. GPL-3.0-or-later.", fontSize = 11.sp, color = Lg.placeholder)
            Spacer(Modifier.height(24.dp))
        }
    }
}

private fun modColor(kind: String) = when (kind) {
    "crafted" -> Color(0xFFB8DAF2); "fractured" -> Color(0xFFA29162); else -> Color(0xFF8888FF)
}

private fun androidx.compose.foundation.lazy.LazyListScope.parsedItems(p: Parsed, s: UiState, vm: LensViewModel) {
    val e = s.edit
    // The item card: same layout as the desktop overlay's header.
    item {
        var showMods by remember { mutableStateOf(false) }
        Column(
            Modifier.fillMaxWidth().background(Lg.window, RoundedCornerShape(8.dp)).border(1.dp, Lg.mid, RoundedCornerShape(8.dp)).padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text((e?.itemClass?.takeIf { it.isNotEmpty() } ?: p.rarity).uppercase(), fontWeight = FontWeight.Bold, fontSize = 13.sp, modifier = Modifier.weight(1f))
                e?.let { Text(if (it.corrupted) "Corrupted" else "Not corrupted", fontSize = 13.sp, color = if (it.corrupted) Lg.red else Dim) }
            }
            Text(p.title, color = Lg.accent, fontWeight = FontWeight.Bold, fontSize = 17.sp)
            @OptIn(ExperimentalLayoutApi::class)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                LgBadge((if (s.game == GameId.POE2) "PoE 2" else "PoE 1") + " · " + s.league)
                LgBadge("${p.rarity} · base ${(p.baseConfidence * 100).toInt()}%")
                e?.let { LgBadge("${it.enabledCount} of ${it.filters.size} filters", accent = true) }
            }
            p.properties.forEach { Text(it, fontSize = 13.sp, color = Dim) }
            if (p.mods.isNotEmpty()) {
                Text("${p.mods.size} mods read from screen · ${s.ocrLineCount} OCR lines ${if (showMods) "▲" else "▼"}", fontSize = 12.sp, color = Lg.placeholder,
                    modifier = Modifier.clickable { showMods = !showMods }.padding(vertical = 2.dp))
            }
            if (showMods) p.mods.forEach { m ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(m.text, Modifier.weight(1f), color = modColor(m.kind), fontSize = 14.sp)
                    if (m.kind != "explicit") Text(m.kind.uppercase(), fontSize = 9.sp, color = Lg.placeholder, modifier = Modifier.padding(horizontal = 4.dp))
                    val c = (m.confidence * 100).toInt()
                    Text("$c%", fontSize = 11.sp, color = if (c >= 90) Lg.green else Color(0xFFFFB74D))
                }
            }
            if (p.unmatched.isNotEmpty()) {
                var open by remember { mutableStateOf(false) }
                Column(Modifier.clickable { open = !open }) {
                    Text("${p.unmatched.size} line(s) not understood ${if (open) "▲" else "▼"}", fontSize = 12.sp, color = Lg.placeholder)
                    if (open) p.unmatched.forEach { Text(it, fontSize = 12.sp, color = Lg.placeholder) }
                }
            }
        }
    }
    s.ninja?.let { n ->
        item {
            Column(Modifier.fillMaxWidth().background(Lg.accent.copy(alpha = 0.18f), RoundedCornerShape(6.dp))
                .border(1.dp, Lg.accent, RoundedCornerShape(6.dp)).padding(horizontal = 10.dp, vertical = 8.dp)) {
                Row(verticalAlignment = Alignment.Bottom) {
                    Text("poe.ninja" + if (s.game == GameId.POE1 && s.realm != RealmId.PC) " (PC)" else "", fontSize = 11.sp, fontWeight = FontWeight.Bold,
                        color = Dim, modifier = Modifier.padding(end = 8.dp, bottom = 2.dp))
                    Text(n.main, fontWeight = FontWeight.Bold, fontSize = 16.sp)
                }
                Text(n.detail, fontSize = 13.sp, color = (if (n.detail.startsWith("▼")) Lg.red else Lg.green).copy(alpha = 0.85f))
            }
        }
    }
    e ?: return
    item {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Tap a row to switch a filter on or off.", fontSize = 12.sp, color = Lg.placeholder, modifier = Modifier.weight(1f))
                if (e.filters.isNotEmpty()) {
                    val allOn = e.enabledCount == e.filters.size
                    TextButton(onClick = { vm.setAllFilters(!allOn) }, contentPadding = PaddingValues(horizontal = 8.dp)) {
                        Text(if (allOn) "None" else "All", fontSize = 13.sp, color = Lg.link)
                    }
                }
            }
            if (e.base != null && e.category != null) {
                ToggleRow("Any ${e.itemClass} (tick for ${e.base} only)".takeIf { !e.useBase } ?: "Base: ${e.base}", e.useBase) { vm.setUseBase(!e.useBase) }
            }
            ToggleRow("Corrupted: " + if (e.corrupted) "yes" else "no", e.filterCorrupted) { vm.setFilterCorrupted(!e.filterCorrupted) }
        }
    }
    itemsIndexed(e.filters) { i, f -> FilterRowView(f, onToggle = { vm.toggleFilter(i) }, onMin = { vm.setFilterMin(i, it) }, onMax = { vm.setFilterMax(i, it) }) }
}

/** A plain checkbox row (base / corruption), 48 dp tall for a thumb. */
@Composable
private fun ToggleRow(label: String, checked: Boolean, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(onClick = onClick).padding(start = 15.dp),
        verticalAlignment = Alignment.CenterVertically) {
        LgCheck(checked)
        Text(label, Modifier.padding(start = 12.dp), fontSize = 15.sp)
    }
}

/**
 * A search filter as a phone-sized row: the whole row is the touch target for on/off, styled like
 * the desktop's filter rows (accent tint + accent bar when on, square tick box), with the min/max
 * fields under the label so long mod text doesn't wrap word by word. Typing a bound switches the
 * filter on.
 */
@Composable
private fun FilterRowView(f: FilterEdit, onToggle: () -> Unit, onMin: (String) -> Unit, onMax: (String) -> Unit) {
    Row(
        Modifier.fillMaxWidth().height(IntrinsicSize.Min).heightIn(min = 56.dp)
            .accentTint(f.enabled)
            .clickable(onClick = onToggle)
            .padding(start = 4.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AccentBar(f.enabled)
        Spacer(Modifier.width(8.dp))
        LgCheck(f.enabled)
        Column(Modifier.weight(1f).padding(start = 12.dp, top = 8.dp, bottom = 8.dp)) {
            Text(f.label, fontSize = 15.sp, color = if (f.enabled) Lg.windowText else Dim)
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(f.tag.uppercase(), fontSize = 10.sp, color = Lg.windowText.copy(alpha = 0.5f), letterSpacing = 0.5.sp,
                    modifier = Modifier.weight(1f))
                NumField(f.min, "min", onMin)
                Spacer(Modifier.width(6.dp))
                NumField(f.max, "max", onMax)
            }
        }
    }
}

@Composable
private fun NumField(value: String, hint: String, onChange: (String) -> Unit) {
    val focus = LocalFocusManager.current
    BasicTextField(
        value = value,
        onValueChange = { v -> onChange(v.filter { it.isDigit() || it == '.' || it == '-' }.take(8)) },
        singleLine = true,
        textStyle = TextStyle(color = Lg.windowText, fontSize = 15.sp, textAlign = TextAlign.Center),
        cursorBrush = SolidColor(Lg.accent),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { focus.clearFocus() }),
        modifier = Modifier.width(58.dp).height(40.dp)
            .background(Lg.base, RoundedCornerShape(4.dp))
            .border(1.dp, Lg.mid, RoundedCornerShape(4.dp)),
        decorationBox = { inner ->
            Box(Modifier.fillMaxSize().padding(horizontal = 4.dp), contentAlignment = Alignment.Center) {
                if (value.isEmpty()) Text(hint, color = Lg.placeholder, fontSize = 13.sp)
                inner()
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsRow(s: UiState, vm: LensViewModel) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Chip("PoE 1", s.game == GameId.POE1) { vm.setGame(GameId.POE1) }
            Chip("PoE 2", s.game == GameId.POE2) { vm.setGame(GameId.POE2) }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            if (s.game == GameId.POE1) {
                Chip("Xbox", s.realm == RealmId.XBOX) { vm.setRealm(RealmId.XBOX) }
                Chip("PlayStation", s.realm == RealmId.SONY) { vm.setRealm(RealmId.SONY) }
                Chip("PC", s.realm == RealmId.PC) { vm.setRealm(RealmId.PC) }
            } else {
                Text("One crossplay trade realm for all platforms", fontSize = 12.sp, color = Lg.placeholder)
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            var open by remember { mutableStateOf(false) }
            Box(Modifier.weight(1f)) {
                Row(Modifier.border(1.dp, Lg.midlight, RoundedCornerShape(4.dp)).background(Lg.button, RoundedCornerShape(4.dp))
                    .clickable { open = true }.padding(horizontal = 10.dp, vertical = 8.dp)) {
                    Text(s.league, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                    Text("  ▾", color = Dim)
                }
                DropdownMenu(open, onDismissRequest = { open = false }, containerColor = Lg.button) {
                    (s.leagues.ifEmpty { listOf(s.league) }).forEach { l ->
                        DropdownMenuItem(text = { Text(l) }, onClick = { vm.setLeague(l); open = false })
                    }
                }
            }
            Text("Online", fontSize = 14.sp, modifier = Modifier.padding(start = 12.dp))
            Switch(s.onlineOnly, vm::setOnlineOnly, Modifier.padding(start = 6.dp),
                colors = SwitchDefaults.colors(checkedTrackColor = Lg.accent, uncheckedTrackColor = Lg.base, uncheckedBorderColor = Lg.midlight, uncheckedThumbColor = Lg.placeholder))
        }
    }
}

/** Selectable chip: the desktop's selected look (accent tint + accent outline). */
@Composable
private fun Chip(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.border(1.dp, if (selected) Lg.accent else Lg.mid, RoundedCornerShape(4.dp))
            .background(if (selected) Lg.accent.copy(alpha = 0.18f) else Color.Transparent, RoundedCornerShape(4.dp))
            .clickable(onClick = onClick).padding(horizontal = 12.dp, vertical = 8.dp)
    ) { Text(label, fontSize = 14.sp, fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal) }
}

/**
 * Camera with zoom, for photographing a TV from the couch: pinch or the slider/presets to zoom
 * (CameraX switches to a telephoto lens where the phone has one), tap to focus. The last zoom is
 * remembered, since the distance to the TV rarely changes.
 */
@Composable
fun CameraScreen(onCaptured: (android.graphics.Bitmap) -> Unit, onClose: () -> Unit) {
    val ctx = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val prefs = remember { ctx.getSharedPreferences("settings", android.content.Context.MODE_PRIVATE) }
    val controller = remember {
        LifecycleCameraController(ctx).apply {
            setEnabledUseCases(CameraController.IMAGE_CAPTURE)
            imageCaptureMode = ImageCapture.CAPTURE_MODE_MAXIMIZE_QUALITY
            isPinchToZoomEnabled = true
            isTapToFocusEnabled = true
        }
    }
    var zoom by remember { mutableStateOf<ZoomState?>(null) }
    DisposableEffect(owner) {
        controller.bindToLifecycle(owner)
        // Restore the last zoom once the camera is up.
        controller.initializationFuture.addListener({
            val saved = prefs.getFloat("zoom", 1f)
            if (saved > 1f) controller.setZoomRatio(saved)
        }, ContextCompat.getMainExecutor(ctx))
        val obs = Observer<ZoomState> { zoom = it }
        controller.zoomState.observe(owner, obs)
        onDispose {
            controller.zoomState.removeObserver(obs)
            controller.unbind()
        }
    }
    val setZoom = { r: Float ->
        controller.setZoomRatio(r)
        prefs.edit().putFloat("zoom", r).apply()
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(factory = { c -> PreviewView(c).also { it.controller = controller } }, modifier = Modifier.fillMaxSize())
        Text("Zoom in until the tooltip fills the frame. Avoid glare; hold steady.",
            Modifier.align(Alignment.TopCenter).systemBarsPadding().padding(16.dp).background(Color(0x99000000)).padding(8.dp), color = Color.White)
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().systemBarsPadding().background(Color(0x99000000)).padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (zoom == null) Text("Starting camera… (no zoom info from the camera yet)", color = Color.Gray, fontSize = 12.sp)
            zoom?.let { z ->
                val max = z.maxZoomRatio
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    listOf(1f, 2f, 5f).filter { it <= max + 0.01f }.forEach { r ->
                        val on = kotlin.math.abs(z.zoomRatio - r) < 0.05f
                        OutlinedButton(onClick = { setZoom(r) }, contentPadding = PaddingValues(horizontal = 12.dp),
                            colors = if (on) ButtonDefaults.outlinedButtonColors(containerColor = Lg.accent.copy(alpha = 0.33f)) else ButtonDefaults.outlinedButtonColors()) {
                            Text("${r.toInt()}×", color = Color.White)
                        }
                    }
                }
                Text("Zoom %.1f× · this camera goes up to %.0f×".format(z.zoomRatio, z.maxZoomRatio), color = Color.White, fontSize = 13.sp)
                if (max > 1.01f) {
                    Slider(
                        value = z.linearZoom,
                        onValueChange = { controller.setLinearZoom(it) },
                        onValueChangeFinished = { prefs.edit().putFloat("zoom", zoom?.zoomRatio ?: 1f).apply() },
                    )
                } else {
                    Text("The phone reports no zoom for this camera", color = Color.Gray, fontSize = 12.sp)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                OutlinedButton(onClick = onClose) { Text("Cancel") }
                Button(onClick = {
                    prefs.edit().putFloat("zoom", zoom?.zoomRatio ?: 1f).apply()
                    controller.takePicture(ContextCompat.getMainExecutor(ctx), object : ImageCapture.OnImageCapturedCallback() {
                        override fun onCaptureSuccess(image: ImageProxy) {
                            val bmp = Ocr.rotate(image.toBitmap(), image.imageInfo.rotationDegrees)
                            image.close()
                            onCaptured(bmp)
                        }
                        override fun onError(e: ImageCaptureException) {
                            android.widget.Toast.makeText(ctx, "Capture failed: ${e.message}", android.widget.Toast.LENGTH_LONG).show()
                        }
                    })
                }) { Text("Capture") }
            }
        }
    }
}
