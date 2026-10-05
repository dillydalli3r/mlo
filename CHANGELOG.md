# Changelog

## 0.1.0 — initial

Native Rust music library manager: TUI (default on a terminal) and scriptable CLI
from one binary.

- **Library & layout** ([§4]): one walk builds the SQLite index (per-file
  mtime+size reuse), a layout audit of 14 finding kinds with re-derived,
  Trash-backed fixes, split-artist merge, and one stored library-wide report.
- **Tagging** ([§6]): FLAC, Ogg Vorbis, Opus and MP3 with full key fidelity
  (foreign keys preserved); MP4/WAV/AIFF map the shared field set and report
  other keys as unplaced. Atomic writes; writer and grader share one
  normalization.
- **Grading** ([§9]): binary, counted verdicts from a single check registry;
  `Not applicable` counts on neither side; any check that raises counts as
  failed; every displayed layout finding is charged on its row.
- **Analysis** ([§8]): TT-DR dynamic range, EBU R128 ReplayGain with true peak,
  BPM and Krumhansl–Schmuckler key detection, FLAC STREAMINFO md5 — all in
  process.
- **Scripts** ([§7]): the 23 passes with a run-all order and a derived import
  chain ending layout → grade; unavailable passes report the reason.
- **Import** ([§5]): acquire (incl. archives) → identify (MusicBrainz) → tag →
  place by the naming template → chain → grade.
- **Trash** ([§1.2]): one implementation, manifests, exact restore.
- **TUI** ([§13]): Library / Album / Artist / Grade / Layout / Scripts / Import /
  Tools / Player / Log / Settings / Trash plus a Library-status dashboard;
  tag view + edit per track; cancellable progress; one status line.
- **Shell integration**: right-click entry on Windows, macOS and Linux via
  `mlo shell install`.
- **Tests** ([§14]): 67 unit tests and 12 acceptance tests, headless, no network,
  no audio device.

### la-musica alignment

The reference implementation is `F:/Coding/GitHub/la-musica`; its
`docs/OPTIMIZATION-GRADING-SPEC.md` is the contract.

- **Check registry** renamed to la-musica's `grade_check_*` / `grade_include_*`
  keys with its 71 labels and defaults (all ON), plus **Strict / Balanced /
  Relaxed presets**.
- **Tag values** use la-musica's exact closed vocabularies (`MEDIA`, `SOURCE`,
  `RELEASETYPE`, `RELEASESTATUS`, `AUDIT`, `MOOD`), its code-shaped rules
  (`RELEASECOUNTRY`, `SCRIPT`), its spacing rule (multi-line values never
  judged) and its `"; "` list separator.
- **Naming** is la-musica's shipped script verbatim (artist/album/release/group
  ids, label, file name with `$num()`), with its sanitiser (trailing dot → `_`,
  reserved device names, empty bracket-group removal).
- **Layout report** is `layout_report.json` in la-musica's document and row
  shape (`scanned_at`, `music_folder`, `report{folder, artists_dir, exists,
  issues, counts, total, albums, artists, audio_files}`; rows carry
  `kind, path, abs, detail, hint, fix?`).
- **Trash** uses la-musica's flat bin with `.mlo_manifest.json`
  (`{version, entries:{name:{origin, at}}}`); the older timestamped layout is
  still listed and restorable.
- **Config** keys match la-musica's names, the grade-affecting set from spec §9
  is modelled, unknown keys are preserved in `[extra]`, and a la-musica
  `config.json` is imported on first run.
- **Scripts** take la-musica's names, descriptions, `SCRIPT_GATES` (ANY-of
  semantics for script 17), opt-in set (`22`) and import chain (equal to the
  run-all order).
- [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md) records every format, and
  `docs/`, `install.sh`/`install.ps1`, `RELEASING.md` and
  `.github/workflows/release.yml` add the Pages site and cross-platform release.