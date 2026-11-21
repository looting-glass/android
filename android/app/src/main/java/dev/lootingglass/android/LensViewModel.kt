package dev.lootingglass.android

import android.app.Application
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.lootlens_ffi.Engine
import uniffi.lootlens_ffi.GameId
import uniffi.lootlens_ffi.NinjaRow
import uniffi.lootlens_ffi.Parsed
import uniffi.lootlens_ffi.PriceRow
import uniffi.lootlens_ffi.RealmId
import org.json.JSONObject
import java.io.File

/** One search filter as the user edits it. Min/max are kept as typed; blank or invalid = no bound. */
data class FilterEdit(val label: String, val tag: String, val enabled: Boolean, val min: String, val max: String)

/** The editable part of the trade search (what the desktop overlay's checkboxes control). */
data class SearchEdit(
    val base: String?,
    val itemClass: String,
    val category: String?,
    val useBase: Boolean,
    val corrupted: Boolean,
    val filterCorrupted: Boolean,
    val filters: List<FilterEdit>,
) {
    val enabledCount get() = filters.count { it.enabled }

    /** Apply the edits to the spec from `parse`, for `search`. */
    fun applyTo(specJson: String, online: Boolean): String {
        val o = JSONObject(specJson)
        o.put("use_base", useBase)
        o.put("filter_corrupted", filterCorrupted)
        o.put("online", online)
        val fs = o.getJSONArray("filters")
        filters.forEachIndexed { i, f ->
            val j = fs.getJSONObject(i)
            j.put("enabled", f.enabled)
            j.put("min", f.min.toDoubleOrNull() ?: JSONObject.NULL)
            j.put("max", f.max.toDoubleOrNull() ?: JSONObject.NULL)
        }
        return o.toString()
    }

    companion object {
        private fun num(o: JSONObject, k: String) =
            if (o.isNull(k)) "" else o.getDouble(k).let { if (it == Math.floor(it)) it.toLong().toString() else it.toString() }

        fun from(specJson: String): SearchEdit {
            val o = JSONObject(specJson)
            val fs = o.getJSONArray("filters")
            return SearchEdit(
                base = o.optString("base").takeIf { !o.isNull("base") && it.isNotEmpty() },
                itemClass = o.optString("class"),
                category = o.optString("category").takeIf { !o.isNull("category") && it.isNotEmpty() },
                useBase = o.optBoolean("use_base"),
                corrupted = o.optBoolean("corrupted"),
                filterCorrupted = o.optBoolean("filter_corrupted"),
                filters = (0 until fs.length()).map { i ->
                    val f = fs.getJSONObject(i)
                    FilterEdit(f.getString("label"), f.getString("tag"), f.getBoolean("enabled"), num(f, "min"), num(f, "max"))
                },
            )
        }
    }
}

data class UiState(
    val game: GameId = GameId.POE1,
    val realm: RealmId = RealmId.XBOX,
    val league: String = "Standard",
    val leagues: List<String> = emptyList(),
    /** Console markets are thin and few sellers show as online: default to any status. */
    val onlineOnly: Boolean = false,
    val busy: String? = null,
    val image: Bitmap? = null,
    val ocrLineCount: Int = 0,
    val parsed: Parsed? = null,
    val edit: SearchEdit? = null,
    /** Searching with edited filters (the old listings stay on screen meanwhile). */
    val searching: Boolean = false,
    val price: PriceRow? = null,
    val ninja: NinjaRow? = null,
    val error: String? = null,
    val dataStatus: String = "",
)

class LensViewModel(app: Application) : AndroidViewModel(app) {
    private val prefs = app.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val engine = Engine(File(app.cacheDir, "trade-data").absolutePath)

    private val _state = MutableStateFlow(
        UiState(
            game = GameId.entries.getOrElse(prefs.getInt("game", 0)) { GameId.POE1 },
            realm = RealmId.entries.getOrElse(prefs.getInt("realm", 1)) { RealmId.XBOX },
            league = prefs.getString("league", "Standard") ?: "Standard",
            onlineOnly = prefs.getBoolean("online", false),
        )
    )
    val state: StateFlow<UiState> = _state

    init {
        refreshLeagues()
        preload()
    }

    private fun save() {
        val s = _state.value
        prefs.edit().putInt("game", s.game.ordinal).putInt("realm", s.realm.ordinal)
            .putString("league", s.league).putBoolean("online", s.onlineOnly).apply()
    }

    fun setGame(g: GameId) {
        _state.update { it.copy(game = g, leagues = emptyList()) }
        save(); refreshLeagues(); preload()
    }

    fun setRealm(r: RealmId) {
        _state.update { it.copy(realm = r, leagues = emptyList()) }
        save(); refreshLeagues()
    }

    fun setLeague(l: String) {
        prefs.edit().putBoolean("league_chosen", true).apply()
        _state.update { it.copy(league = l) }; save()
    }
    fun setOnlineOnly(v: Boolean) {
        _state.update { it.copy(onlineOnly = v) }; save()
        if (_state.value.edit != null) scheduleSearch(0)
    }

    // ── Filter editing: every change re-runs the search after a short pause, like the desktop overlay.
    private var searchJob: Job? = null

    fun toggleFilter(i: Int) = editSearch { e -> e.copy(filters = e.filters.mapIndexed { j, f -> if (j == i) f.copy(enabled = !f.enabled) else f }) }
    fun setFilterMin(i: Int, v: String) = editSearch(1200) { e -> e.copy(filters = e.filters.mapIndexed { j, f -> if (j == i) f.copy(min = v, enabled = true) else f }) }
    fun setFilterMax(i: Int, v: String) = editSearch(1200) { e -> e.copy(filters = e.filters.mapIndexed { j, f -> if (j == i) f.copy(max = v, enabled = true) else f }) }
    fun setUseBase(v: Boolean) = editSearch { it.copy(useBase = v) }
    fun setFilterCorrupted(v: Boolean) = editSearch { it.copy(filterCorrupted = v) }
    fun setAllFilters(on: Boolean) = editSearch { e -> e.copy(filters = e.filters.map { it.copy(enabled = on) }) }

    private fun editSearch(delayMs: Long = 700, f: (SearchEdit) -> SearchEdit) {
        if (_state.value.edit == null) return
        _state.update { it.copy(edit = it.edit?.let(f)) }
        scheduleSearch(delayMs)
    }

    private fun scheduleSearch(delayMs: Long) {
        searchJob?.cancel()
        searchJob = viewModelScope.launch {
            delay(delayMs)
            val s = _state.value
            val p = s.parsed ?: return@launch
            val e = s.edit ?: return@launch
            _state.update { it.copy(searching = true, error = null) }
            try {
                val price = engine.search(e.applyTo(p.specJson, s.onlineOnly), 10u)
                _state.update { it.copy(price = price, searching = false) }
            } catch (ex: kotlinx.coroutines.CancellationException) {
                throw ex
            } catch (ex: Exception) {
                Log.w(TAG, "search failed", ex)
                _state.update { it.copy(error = ex.message ?: ex.toString(), searching = false) }
            }
        }
    }

    private fun preload() = viewModelScope.launch {
        val g = _state.value.game
        try {
            val st = engine.load(g)
            _state.update { it.copy(dataStatus = "Trade data: $st") }
        } catch (e: Exception) {
            _state.update { it.copy(dataStatus = "Trade data: ${e.message}") }
        }
    }

    private fun refreshLeagues() = viewModelScope.launch {
        val s = _state.value
        try {
            val ls = engine.leagues(s.game, s.realm)
            _state.update { cur ->
                // Until the user picks one, use the current softcore challenge league (listed first).
                val chosen = prefs.getBoolean("league_chosen", false) && cur.league in ls
                val league = if (chosen) cur.league else ls.firstOrNull { !it.contains("Hardcore") && !it.startsWith("HC") && !it.contains("Ruthless") } ?: "Standard"
                cur.copy(leagues = ls, league = league)
            }
            save()
        } catch (e: Exception) {
            Log.w(TAG, "leagues: ${e.message}")
        }
    }

    /** Photo or picked image -> OCR -> parse -> trade search (+ poe.ninja for stackables). */
    fun check(bitmap: Bitmap) = viewModelScope.launch {
        val s = _state.value
        searchJob?.cancel()
        _state.update { it.copy(image = bitmap, parsed = null, edit = null, searching = false, price = null, ninja = null, error = null, busy = "Reading text…") }
        try {
            val lines = Ocr.recognize(bitmap)
            _state.update { it.copy(ocrLineCount = lines.size, busy = "Matching mods…") }
            engine.load(s.game)
            val parsed = withContext(Dispatchers.Default) {
                engine.parse(s.game, s.realm, s.league, s.onlineOnly, lines)
            }
            val edit = SearchEdit.from(parsed.specJson)
            _state.update { it.copy(parsed = parsed, edit = edit, busy = "Searching ${realmLabel(s)}…") }
            parsed.stackableName?.let { name ->
                launch {
                    val n = engine.ninja(s.game, s.league, name, parsed.stackSize)
                    _state.update { it.copy(ninja = n) }
                }
            }
            val price = engine.search(edit.applyTo(parsed.specJson, s.onlineOnly), 10u)
            _state.update { it.copy(price = price, busy = null) }
        } catch (e: Exception) {
            Log.w(TAG, "check failed", e)
            _state.update { it.copy(error = e.message ?: e.toString(), busy = null) }
        }
    }

    /**
     * Test-set mode (`adb shell am start ... --ez batch true`): OCR every image in
     * <filesDir>/testset/ and write `<name>.ocr.json` (+ `<name>.parsed.txt`) to files/ocr/, for the
     * desktop eval harness. No trade searches.
     */
    fun runBatch() = viewModelScope.launch {
        val app = getApplication<Application>()
        // Internal storage: reachable with `adb shell run-as` on debug builds on every Android version.
        val inDir = File(app.filesDir, "testset")
        val outDir = File(app.filesDir, "ocr").apply { mkdirs() }
        val files = inDir.listFiles { f -> f.extension.lowercase() in setOf("png", "jpg", "jpeg") }?.sortedBy { it.name } ?: emptyList()
        _state.update { it.copy(busy = "Batch: ${files.size} images") }
        val log = StringBuilder()
        for ((i, f) in files.withIndex()) {
            _state.update { it.copy(busy = "Batch ${i + 1}/${files.size}: ${f.name}") }
            try {
                val bmp = withContext(Dispatchers.IO) { BitmapFactory.decodeFile(f.absolutePath) }
                val t0 = System.currentTimeMillis()
                val lines = Ocr.recognize(bmp)
                val t1 = System.currentTimeMillis()
                File(outDir, "${f.nameWithoutExtension}.ocr.json").writeText(Ocr.toJson(lines))
                val game = if (f.name.contains("poe2")) GameId.POE2 else GameId.POE1
                engine.load(game)
                val parsed = runCatching { engine.parse(game, RealmId.PC, "Standard", false, lines) }
                val t2 = System.currentTimeMillis()
                File(outDir, "${f.nameWithoutExtension}.parsed.txt").writeText(
                    parsed.fold({ p -> p.clipboard + "\n# unmatched: " + p.unmatched.joinToString(" | ") }, { e -> "ERROR ${e.message}" })
                )
                log.append("${f.name}\t${lines.size} lines\tocr ${t1 - t0} ms\tparse ${t2 - t1} ms\n")
            } catch (e: Exception) {
                log.append("${f.name}\tERROR ${e.message}\n")
            }
        }
        File(outDir, "done.txt").writeText(log.toString())
        _state.update { it.copy(busy = null, error = "Batch done: ${files.size} images → ${outDir.absolutePath}") }
    }

    companion object {
        const val TAG = "LootingGlass"
        fun realmLabel(s: UiState) = when {
            s.game == GameId.POE2 -> "PoE 2 (crossplay)"
            s.realm == RealmId.XBOX -> "Xbox"
            s.realm == RealmId.SONY -> "PlayStation"
            else -> "PC"
        }
    }
}
