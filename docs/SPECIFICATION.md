# mlo — Data & Format Specification

**Status:** contract. This document specifies every on-disk and wire format the
app reads or writes. It is derived from the reference implementation
**la-musica** (its `docs/OPTIMIZATION-GRADING-SPEC.md` and the `mlo/` and
`server/` code), so a library written by either app is understood identically.
Where a value is named here, it is required, not illustrative.

Sections marked **mlo extension** describe behaviour this app adds on top of
la-musica (the Rust brief requires it); everything else mirrors la-musica
verbatim. Inline citations of the form `la-musica:<file>:<line>` point at the
source of the format.

---

## 0. Vocabulary

| Term | Meaning |
|---|---|
| library root | `music_folder` — the folder the app owns |
| app state | `<library root>/.mlo/` — never library content |
| artist folder | a directory directly under `<library root>/Artists` |
| album folder | a directory directly under an artist folder |
| track | an audio file inside an album folder |
| sidecar | a non-audio file the app treats as part of an album |
| check | one enabled assertion evaluated by the grader |
| issue | `{code, label, where, reason}` raised by a failing check |
| finding | a library-layout problem (`{kind, path, abs, detail, hint, fix?}`) |

Canonical library shape: `<library root>/Artists/<Artist>/<Album>/<files>`
(`la-musica:mlo/layout.py:1-4`).

---

## 1. Directory layout

```
<music_folder>/
  Artists/
    <Artist>/                       # artist folder
      artist.jpg | artist.png       # artist image (either)
      description.txt               # artist description (UTF-8)
      <Album>/                      # album folder
        <tracks>                    # audio
        cover.jpg|cover.jpeg|cover.png|cover.jxl   # album cover sidecar
        description.txt             # album description (UTF-8)
        <track-stem>.lrc            # lyrics (synced or plain)
        *.cue  *.log  *.accurip     # disc sidecars
        .mlo_expected.json          # release tracklist manifest
        .mlo_covers.json            # per-track cover manifest (hidden)
        .mlo_pending.json           # pending-work manifest (hidden)
        <disc>/                     # CD1 / Disc 2 / … (optional)
  .mlo/                             # APP STATE — never library content
    data/                           # config + index + reports + caches
    tools/<Name> v<version>/        # installed external tools
    downloads/  incomplete/         # acquisition scratch
    trash/<user>/.mlo_manifest.json # removal bin + origin manifest
```

- `Artists/` holds only artist folders (plus nothing else).
- An album folder holds audio, `cover.*`, `description.txt`, sidecars, and
  optional disc folders.
- A disc folder is named `CD<n>`/`Disc <n>` (`is_disc_folder`); `VIDEO_TS`/`BDMV`
  are disc *structures* remuxed into one title, not disc folders.

---

## 2. Configuration

### 2.1 File and location

| | |
|---|---|
| This app's file | `mlo.toml` in the platform config dir (`%APPDATA%\mlo\mlo.toml`, `~/.config/mlo/mlo.toml`, `~/Library/Application Support/mlo/mlo.toml`) |
| la-musica's file | `<music_folder>/.mlo/data/config.json`, else `<install dir>/config.json` (`la-musica:mlo/config.py:2165`) |
| Compatibility | on first run, `mlo.toml` is created by importing a la-musica `config.json` found in the config dir, the CWD, or `$MLO_MUSIC_FOLDER/.mlo/data/config.json`. **The original file is never rewritten in place.** |
| Encoding | UTF-8; JSON in la-musica is written sorted, indent 2, atomically |
| Unknown keys | preserved verbatim under the `[extra]` table, so a round trip never drops a setting this build does not model |

### 2.2 Key names

Every key keeps la-musica's exact spelling — a config copied between the two
apps is read by name. The full set of la-musica keys that change a grade is
`la-musica:docs/OPTIMIZATION-GRADING-SPEC.md` §9; the modelled subset:

- **Checks** — `grade_check_*` (62 keys) and `grade_include_*` (9 file-category
  keys). **Every check ships ON** (`la-musica:mlo/config.py`; R18).
- **Grade thresholds** — `grade_verbose` (ON), `grade_log_score_threshold` (100),
  `grader_cover_size_tolerance_px` (0), `grader_strict_square_threshold` (0.0),
  `cover_crop_threshold` (0.0).
- **Cover policy** — `cover_resize_enabled`, `cover_target_size` (1200),
  `cover_force_exact_size`, `cover_enforce_size`, `cover_enforce_square`,
  `cover_crop_enabled`, `cover_jpeg_target_size`/`cover_png_target_size`/
  `cover_jxl_target_size` (0 = the global target), `cover_country` (`us`),
  `cover_sources`, `reencode_images`, `embed_covers` (off),
  `embed_cover_jpeg_quality` (90), `embed_cover_resolution` (1200).
- **Artist artefacts** — `artist_image_enabled`, `artist_image_aspect` (`1:1`),
  `artist_image_crop`, `artist_image_target_size` (0 = provider-native, ceiling
  2000 px), `artist_description_enabled`, `album_description_enabled`,
  `description_full`, `description_sources`, `artist_image_sources`.
- **Genres** — `mb_genre_count` (2, hard ceiling 3), `genre_autofill`,
  `genre_sources` (priority list: rateyourmusic, musicbrainz, listenbrainz,
  itunes, lastfm, theaudiodb, wikidata, bandcamp, discogs, deezer, spotify).
- **Audio** — `audiometa_enabled`, `audiometa_key_notation` (`musical`),
  `dr_replaygain_enabled`, `write_replaygain_tags`, `write_dynamic_range_tags`,
  `replaygain_skip_existing`, `force_dr_replaygain`, `mood_enabled`,
  `mood_source` (`hybrid`).
- **Lyrics** — `lyrics_format` (`EMBEDDED`), `lyrics_allow_plain` (off),
  `lyrics_translation_langs` (`en`), `optimize_lrc`, `optimize_embedded_lyrics`,
  `lrc_timestamp_precision` (2), `lrc_strip_metadata`, `lrc_collapse_blank_lines`,
  `lrc_enhanced_enabled`, `lrc_enhanced_word_sync`, `lrc_sync_level` (`LINE`),
  `lrc_extended_enabled`, `lrc_add_zero_timestamp` (off),
  `lrc_zero_timestamp_blank` (off), `lrc_zero_timestamp_target` (`BOTH`),
  `lyrics_xlit_enabled`, `lyrics_translate_enabled`.
- **Text sidecars** — `append_final_newline` (off), `keep_empty_cue_lines` (off),
  `keep_other_cue_lines` (off), `keep_empty_accurip_lines` (off),
  `cue_file_type` (`WAVE`), `discs_rename_enabled`, `discs_rename_pattern`
  (`CD-{n}`), `discs_rename_single_fallback`, `cue_fix_filenames`,
  `discs_toc_tolerance_s` (4.0), `discs_toc_unique_margin_s` (4.0).
- **Tags** — `strip_unknown_tags` (ON), `normalize_media_source` (ON),
  `digital_media_source_value` (`Digital`), `fill_empty_source` (off),
  `strip_source_on_cd` (ON), `audio_tag_writes` (per-format map, unmodelled —
  preserved in `[extra]`).
- **Audit** — `audit_require_accuraterip`, `audit_verify_log_checksum`,
  `audit_check_cd_format`, `audit_verify_cd_checksums`, `audit_integrity`,
  `audit_cd_require_both`, `audit_log_score_threshold` (100),
  `audit_fail_on_unscorable_log`, `audit_thorough`, `audit_mqa`, `audit_ai`,
  `write_audit_tag`, `write_log_grade`.
- **Naming / layout** — `naming_script`, `short_folder_names`, `layout_apply`.
- **Plan** — `run_all_order` (empty = shipped order), `import_scripts`,
  `import_auto_scripts`, `library_codec` (`flac`), `library_codec_quality` (5),
  `library_codec_bitrate` (0), `library_codec_args`, `library_codec_optimize`
  (`lossless_to_lossy`), `lossless_remove_original`, `video_remove_original`.
- **AcoustID / web** — `acoustid_enabled`, `fingerprint_submit_enabled`
  (la-musica: `acoustid_enabled` gates scripts 21 and 22),
  `web_ratings_enabled`, `ai_effort` (`high`), `ai_genre_effort` (`high`).

### 2.3 Grading presets (`la-musica:docs/OPTIMIZATION-GRADING-SPEC.md` §4)

| Preset | Effect |
|---|---|
| **Strict** | the shipped defaults: every `grade_check_*` and `grade_include_*` true. The identity preset. |
| **Balanced** | defaults with `grade_check_audit` and `grade_include_other` **off**. |
| **Relaxed** | defaults with these 18 keys **off**: `grade_check_tag_spaces`, `grade_check_tag_case`, `grade_check_lyrics_spaces`, `grade_check_cue_spaces`, `grade_check_cover_crop`, `grade_check_lyrics_zero`, `grade_check_tag_blank_lines`, `grade_check_lyrics_blank_lines`, `grade_check_cue_blank_lines`, `grade_check_filename_case`, `grade_check_ext_case`, `grade_check_excess_tags`, `grade_check_mb_links`, `grade_check_rym_links`, `grade_check_replaygain`, `grade_check_album_description`, `grade_check_artist_image`, `grade_check_artist_description`. |

A preset edits the local config and only takes effect on **save**. In the TUI:
the Settings screen, keys `S` / `B` / `R`, then `s` to save.

---

## 3. Index database (mlo)

`<music_folder>/.mlo/data/index.db` — SQLite, WAL, `busy_timeout` 10 s.

| table | columns |
|---|---|
| `library` | `path PK, kind, artist, album, track, disc, container, duration_ms, mtime, size, quick_hash, indexed_at, has_cover, has_lyrics, has_cue, has_log, has_accurip, grade_pct, grade_pass` |
| `tags` | `path, key, value` |
| `albums` | `id PK, artist, title, mb_release_id, mb_release_group_id, path, grade_pct, pass` |
| `artists` | `name PK, path, mb_artist_id, grade_pass, has_image, has_description` |
| `queue` | `pos PK, path, added_at` |
| `player_state` | `id=1, path, position_ms, volume, shuffle, repeat, updated_at` |
| `cache` | `service, key PK(service,key), body, fetched_at, ttl_s` |
| `jobs` | `id PK, kind, scope, state, started_at, finished_at, journal` |
| `trash` | `id PK, original_path, trash_path, user, stamp, reason` |

`quick_hash` is `xxh3-64(first 64 KiB)` + `:size`, used only for change
detection. la-musica's own stores (`auth.db`, `playlists.db`, `plays.db`,
`ratings.db`, `beets-library.db`, `tagindex.sqlite`, `events.jsonl`) are its
server's; this app does not read or write them (the Rust brief §3.6 says
`auth.db`, `beets-library.db` and `events.jsonl` are ignored), and never
converts them.

---

## 4. Layout report

**Path:** `<music_folder>/.mlo/data/layout_report.json`
(`la-musica:mlo/layout.py:1333`). Written **only by a library-wide scan**; a
scoped run stores nothing (`la-musica:mlo/layout.py:1441`).

```json
{
  "scanned_at": "2026-10-05T07:18:57Z",
  "music_folder": "F:/Media/Music",
  "report": {
    "folder": "F:/Media/Music",
    "artists_dir": "F:/Media/Music/Artists",
    "exists": true,
    "issues": [
      { "kind": "wrong_case",
        "path": "Artists/a-b/album two",
        "abs": "F:/Media/Music/Artists/a-b/album two",
        "detail": "'a-b/album two' should be 'A-B/Album Two'",
        "hint": "…",
        "fix": { "action": "rename", "to": "F:/Media/Music/Artists/A-B/Album Two" } }
    ],
    "counts": { "wrong_case": 1 },
    "total": 1,
    "albums": 12,
    "artists": 4,
    "audio_files": 140
  }
}
```

- `fix` is present **only** when the scan worked out what may act on the row; it
  is re-proved at the move (`_within`, `_UNFIXABLE`). `action` is
  `rename` | `move` | `trash`.
- `counts` maps finding kind → count; `path` is library-relative with `/`;
  `abs` is absolute.
- Atomic write (temp + fsync + rename); an unreadable file reads as
  `{"exists": false}`, never as "a scan ran".

### 4.1 Finding kinds (`la-musica:mlo/layout.py`)

| kind | meaning | fix |
|---|---|---|
| `audio_at_root` | audio directly in the library root | move into the album its tags name |
| `audio_in_artists` | audio directly in `Artists/` | move into its album folder |
| `audio_in_artist` | audio directly in an artist folder | move into its album folder |
| `unexpected_folder` | foreign folder in the library root | trash **only** if it holds no audio |
| `unexpected_subfolder` | folder in an album that is neither a disc folder nor holds audio | trash |
| `empty_album` | album folder with no audio beneath | trash |
| `empty_artist` | artist folder with no album **and no sibling for the same artist** | trash |
| `split_artist` | two folders name one artist | merge (artefacts first, then trash the emptied half); **no fix** when both hold albums |
| `wrong_case` | name differs from the naming script by case only | rename |
| `sidecar_copy` | numbered/duplicate sidecar | rename, or trash when the canonical name exists |
| `stray_file` | non-audio, non-sidecar, non-artwork file inside an album | trash |
| `stray_in_artists` | non-audio file directly in `Artists/` | trash |
| `hidden_folder` | hidden folder inside `Artists/` | report only |
| `legacy_state_file` | leftover of the old `.mlo_data` layout | trash |

Two things are never removed: a foreign root folder that holds audio, and a
hidden folder inside `Artists/`.

---

## 5. Grade report

Grading is **binary and counted** (`la-musica:mlo/grader.py`):

- **R1** an album is `PASS` iff every enabled check on every track, file and
  album slot passed; otherwise `FAIL` with the failed checks itemised.
- **R1a** every issue a verdict lists costs at least one failed check — the dot,
  the percentage and the problem list can never disagree.
- **R2** `total_checks` counts every enabled assertion evaluated;
  `pass_count = max(0, total_checks − failed_checks)`; a report prints
  `pass_count/total_checks`.
- **R3** a check that raises is reported *could not be evaluated*, counted and
  failed.
- **R4** a check that does not apply is counted on **neither** side (disabled,
  `VIDEO_SKIP_TAGS`, an opt-in family with no member tags, artist checks on an
  album, an album with no MusicBrainz release id).
- **R5** the run's summary is `albums_passed`/`albums_failed`; a passing album
  whose live audit verdict is FAKE/Mix is counted in `albums_audit_failed` and
  badged `FAIL` on the Audit column. **AccurateRip never costs a grade point by
  itself.**
- **R6** empty folders are graded, not skipped (`EMPTY_FOLDER`, one failed check).
- **R7** artist folders have their own grade; an artist folder holding no album
  at all fails as `ARTIST_EMPTY` (`la-musica:mlo/grader.py:4765`).

### 5.1 Issue codes

`UNREADABLE`; the presence codes `TITLE ARTIST ALBUM ALBUMARTIST DATE
TRACKNUMBER DISCNUMBER GENRE MOOD ENERGY ITUNESADVISORY INSTRUMENTAL
DYNAMIC RANGE REPLAYGAIN_* INITIALKEY BPM MEDIA SOURCE ENCODER_* ACOUSTID_ID
ACOUSTID_FINGERPRINT`; `MOOD_MISSING ENERGY_MISSING GENRE_MISSING`;
`GENRE_COUNT GENRE_ORDER GENRE_VOCAB GENRE_CASE`; `TAGS`; `PATH PATH_CASE`;
`LYRICS`; `XLIT_MISSING XLIT_UNNEEDED`; `MB_LINK RYM_LINK`; `COVER`;
`CRC CRC_MISMATCH`; `CD_FORMAT`; `LOG_CHECKSUM`; `AUDIT`;
`EMPTY_FOLDER EXPECTED_TRACKS_MISSING EXPECTED_TRACKS_INCOMPLETE`;
`ARTIST_IMAGE_MISSING ARTIST_IMAGE_CORRUPT ARTIST_IMAGE_FORMAT
ARTIST_IMAGE_OVERSIZED ARTIST_IMAGE_ASPECT ARTIST_IMAGE_UPSCALED
ARTIST_DESCRIPTION_MISSING ARTIST_FOLDER_MISSING ARTIST_EMPTY`;
`ARTIST_IMAGE_UNDERSIZED` (note, never failing); `COMMENT` (a non-empty
`COMMENT`), `LOG_GRADE`, `FLAC_MD5 FLAC_MD5_ABSENT FLAC_MD5_UNKNOWN`,
`DISALLOWED_FILE`, `EXT_CASE`, `RAW_VIDEO`, `ACCURATERIP`, `ACCURIP_FORMAT`,
`ALBUM_DESCRIPTION`, `LYRICS_SPACES LYRICS_BLANK_LINES LYRICS_ZERO
LYRICS_LANG_TAGS TAG_SPACES TAG_CASE TAG_BLANK_LINES EXTRA_IMAGES
SIDECAR_COVER COVER_CROP`, and the **mlo extension** layout codes
`LAYOUT_<KIND>` (one per §4.1 kind), charged on the album/artist/library row.

### 5.2 Library verdict

```json
{ "albums_total": 12, "albums_passed": 9, "albums_failed": 3,
  "albums_audit_failed": 1, "artists_total": 4, "artists_passed": 2,
  "artists_failed": 2, "tracks_total": 140, "library_row": { … } }
```

---

## 6. Tags

### 6.1 Families (`la-musica:server/tags_registry.py`)

`identity`, `release`, `audio`, `lyrics`, `provenance`. Anything outside them is
foreign and fails `grade_check_excess_tags` unless on the allowlist. Alias tags
accept a locale suffix (`TITLEALIAS_JA`) and are matched by their bare key.

### 6.2 Closed vocabularies — the canonical spelling

A value inside a vocabulary resolves case-insensitively to the spelling below;
a value **outside** it is returned unchanged (never coerced), which is what
makes the rule idempotent (`la-musica:mlo/tagtext.py:154`).

- `MEDIA` — `12" Vinyl, 10" Vinyl, 7" Vinyl, 8-Track, Blu-ray, Blu-spec CD,
  Cassette, CD, CD-R, Digital Media, DVD, DVD-Audio, DVD-Video, HDCD, LaserDisc,
  Minidisc, SACD, SHM-CD, VHS, Vinyl, Web`
- `SOURCE` — `Soulseek, Digital, YouTube`
- `RELEASETYPE` — `Album, EP, Single, Broadcast, Other, Compilation, Soundtrack,
  Spokenword, Interview, Audiobook, Live, Remix, DJ-mix, Mixtape/Street, Demo,
  Audio drama, Field recording, Podcast`
- `RELEASESTATUS` — `Official, Promotion, Bootleg, Pseudo-Release, Withdrawn,
  Expired, Cancelled`
- `AUDIT` — `REAL, FAKE, MIX`
- `MOOD` — `Happy, Energetic, Aggressive, Sad, Calm, Dreamy, Dark, Party`
- `RELEASECOUNTRY` — upper-cased when it is two ASCII letters (`us` → `US`)
- `SCRIPT` — title-cased when it is four ASCII letters (`latn` → `Latn`)
- `COMMENT` — must be empty; a non-empty value fails with code `COMMENT`.

Free text (`TITLE`, `ALBUM`, `ARTIST`, `ALBUMARTIST`, `LABEL`, `COMMENT`, the
lyrics) is untouched, byte for byte — `AC/DC` and `k.d. lang` survive a write.

### 6.3 Spacing (`spacing_problem`)

A leading/trailing space or tab, or a run of two or more **internal** spaces, is
wrong. A multi-line value (any value carrying `\n`/`\r`, and always `LYRICS`,
`UNSYNCEDLYRICS`, `SYNCLYRICS`) is **never judged and never collapsed** — its
whitespace is the text. The writer collapses what the check reports
(`collapse_spacing(canonical_value(tag, value.strip(" \t")))`), so a writer can
never produce a value the grade would fail.

### 6.4 Multi-value fields (`la-musica:mlo/tagtext.py:231`)

A multi-value tag is stored as **repeated container fields**; the reader joins
them with exactly `"; "` (`_LIST_SEP`) — a `;` inside a URL is a character, not
a separator. A container that holds one string per key (a video/MKV) stores the
`"; "`-joined value. A writer **completes** a short list (case-insensitively) and
never truncates it. `RELEASECOUNTRY` and `LABEL` reduce to their **first** value
for the naming path, so one album always yields exactly one deterministic path.

### 6.5 Default writers (`la-musica:docs/OPTIMIZATION-GRADING-SPEC.md` §6)

| tag | writer |
|---|---|
| identity/release core, credits, links | Beets tagging (14) · import |
| `MEDIA`, `SOURCE` | Format lyrics (1) |
| `GENRE` | Auto tagging (8) · genre import; Format all (10) trims |
| `ITUNESADVISORY`, `ALBUMITUNESADVISORY`, `INSTRUMENTAL` | Auto tagging (8) / advisory fetch |
| `MOOD`, `ENERGY` | Auto tagging (8) · Mood & Energy (16) |
| `BPM`, `INITIALKEY` | Key & BPM (12) |
| `DYNAMIC RANGE`, `ALBUM DYNAMIC RANGE`, `REPLAYGAIN_*` | DR & ReplayGain (7) |
| `AUDIT`, `LOG_GRADE`, `LOG_CRC`, `INTEGRITY` | Audit library (6) |
| `ACOUSTID_ID`, `ACOUSTID_FINGERPRINT` | the import wizard's AcoustID apply · Fix AcoustID pairs (21) |
| `ENCODER_PROGRAM`, `ENCODER_QUALITY`, `ENCODER_VERSION` | Optimize FLACs (3); Process images (5) for images |
| `LYRICS`, `UNSYNCEDLYRICS` | Fetch lyrics (13) · the lyrics editor |
| `TRANSLITERATION`, `TRANSLATION` | Lyrics transliterate (AI) (17) |
| `AUDIO_MD5` | nothing — legacy, read only |
| `AUDIOAUDITOR_OVERRIDE` | the track editor (manual); wins over every derived verdict |

---

## 7. Sidecars and special files

| name | meaning |
|---|---|
| `cover.jpg` / `cover.jpeg` / `cover.png` / `cover.jxl` | album cover (front). `front.*` and `folder.*` are recognised as covers too (`la-musica:mlo/format_all.py:47-48`) |
| `description.txt` | album (or artist) description, UTF-8 |
| `artist.jpg` / `artist.png` | artist image (artist folder only) |
| `*<stem>.lrc` | lyrics for the track whose stem matches; synced (timestamps) or plain |
| `*.cue` | CD layout, canonical form (§8.2) |
| `*.log` | rip log (EAC/XLD), graded and checksummed |
| `*.accurip` | AccurateRip evidence (§8.3) |
| `.mlo_expected.json` | release tracklist manifest (missing/extra tracks) |
| `.mlo_covers.json` | per-track cover manifest (hidden) |
| `.mlo_pending.json` | pending-work manifest (hidden) |

A numbered copy of a sidecar (`description (2).txt`) is its own finding
(`sidecar_copy`): renamed when the canonical name is absent, moved to Trash when
it is present.

---

## 8. Text formats enforced by the passes

### 8.1 `.lrc`

One `[mm:ss.xx]` per line, precision `lrc_timestamp_precision` (2 digits).
`lrc_strip_metadata` removes non-timestamp header lines; `lrc_collapse_blank_lines`
folds runs of blank lines; a zero timestamp is governed by
`lrc_add_zero_timestamp` / `lrc_zero_timestamp_blank` /
`lrc_zero_timestamp_target` (`BOTH`); word-sync and stacked timestamps are
governed by `lrc_enhanced_enabled` / `lrc_enhanced_word_sync` /
`lrc_sync_level` (`LINE`) / `lrc_extended_enabled`.

### 8.2 `.cue`

The canonical sheet keeps `REM DISCID`, `FILE`, `TRACK` and `INDEX`
(`la-musica:mlo/cue.py:27-112`); every other directive (`REM` other,
`SONGWRITER`, `PREGAP`, `POSTGAP`, `FLAGS`, …) is dropped unless
`keep_other_cue_lines` is on. `FILE "<name>" <TYPE>` is re-emitted with the
quoted track file and `cue_file_type` (`WAVE`|`MP3`, else `WAVE`), with
`cue_fix_filenames` correcting the referenced name. `INDEX 01` is the app's
canonical start; a split also carries `INDEX 00`.

### 8.3 `.accurip`

CUETools-generated: an `[AccurateRip ID: <…>]` line plus a per-track
`[ CRC | V2 ]` table derived wholly from decoding the audio and querying
AccurateRip/CTDB — never from the rip log's Copy CRC
(`la-musica:mlo/accurip.py:1-27`). An absent file is not a failure; a stale one
is reported.

---

## 9. Naming template

### 9.1 Shipped template (`la-musica:mlo/naming.py:37`)

```
%albumartist% [%musicbrainz_albumartistid%]/$if(%releasetype%,[%releasetype%] ,)$if(%originaldate%,%originaldate% - ,)$if(%date%,%date% - ,)%album% {$if(%releasecountry%,%releasecountry%)$if(%media%,$if(%releasecountry%, - ,)%media%)$if(%catalognumber%,$if(%media%, - ,$if(%releasecountry%, - ,))%catalognumber%)}$if(%label%, [%label%])$if(%musicbrainz_albumid%, [%musicbrainz_albumid%])$if(%musicbrainz_releasegroupid%, [%musicbrainz_releasegroupid%])/%discnumber%-$num(%tracknumber%,2) %title%$if(%musicbrainz_trackid%, [%musicbrainz_trackid%])$if(%musicbrainz_releasegroupid%, [%musicbrainz_releasegroupid%])
```

Grammar: `%field%` substitution; `$if(cond,then[,else])`; `$num(%field%,N)`
zero-padding; `[...]`/`{...}` optional groups dropped whole when a field inside
is absent; `/` is the folder separator. Values are substituted through
sanitisation, so `AC/DC` names one file, never two levels.

### 9.2 Determinism and acceptance

- The path (folders included) equals the evaluated template (R31).
- `short_folder_names` truncates each UUID to 8 characters; **grading accepts
  both spellings** (R32).
- Multi-value `RELEASECOUNTRY`/`LABEL` keep their first value (R33).
- A missing tag with a cold cache is a wildcard: reported as *Missing X tag*,
  never invented as a path, and never triggers a network call (R34).
- Case is part of the contract: a case-only difference fails as `PATH_CASE`
  (`grade_check_filename_case`), and `grade_check_ext_case` requires lowercase
  extensions (R35).

### 9.3 Sanitisation (`sanitize_segment` / `sanitize_path`)

1. Characters `<>:"/\|?*` and ASCII control `\x01-\x1f` become `_` — one for one.
2. Runs of whitespace collapse to a single space; a blank-only name becomes `""`
   (not a name).
3. Leading spaces are stripped.
4. A trailing run of dots/spaces becomes the same **number** of `_`.
5. Windows reserved device names (`CON PRN AUX NUL COM1-9 LPT1-9`, with or
   without extension, any case) get `_` appended to the stem.
6. `sanitize_path` first removes empty `[]`/`{}` groups together with the space
   before them, then sanitises each `/`-separated segment and drops empties.
7. The result is a fixed point: organising an already-organised library is a
   no-op.

---

## 10. Trash bin and manifest

```
<music>/.mlo/trash/<user>/.mlo_manifest.json
```

`<user>` is the OS user, or the literal `default` for an unclaimed install.
Entries sit **directly** in the bin; a name collision gets the `"(2)"` suffix a
file keeps its extension for (`song (2).flac`) and a folder does not
(`name (2)`). Manifest (`la-musica:mlo/paths.py:294-296`):

```json
{ "version": 1,
  "entries": { "<entry name>": { "origin": "F:/Media/Music/Artists/…/stray.nfo",
                                 "at": "2026-10-05T07:18:57" } } }
```

- A missing or corrupt manifest reads as empty; the file is still in the bin.
- The manifest is written atomically after each move.
- Restore puts an entry back to its exact `origin`; a target that already exists
  is refused with a named reason.
- The older timestamped layout (`<user>/<stamp>/manifest.json`) is still listed
  and restorable.
- The guard refuses to trash the music root, `Artists/`, or anything under
  `.mlo/`, and refuses a foreign root folder that gained audio since the scan.

---

## 11. Scripts, order and chains

### 11.1 The 23 passes (`la-musica:mlo/scripts.py:17`)

| # | Name | Switch that gates it |
|---|---|---|
| 1 | Format lyrics | — |
| 2 | Format CUEs | — |
| 3 | Optimize FLACs | — |
| 4 | Grade | — |
| 5 | Process images | — |
| 6 | Audit library | — |
| 7 | DR & ReplayGain | `dr_replaygain_enabled` |
| 8 | Auto tagging | — |
| 9 | AccurateRip | — |
| 10 | Format all | — |
| 11 | Remux videos (MKV) | `video_remux_enabled` |
| 12 | Key & BPM | `audiometa_enabled` |
| 13 | Fetch lyrics | — |
| 14 | Beets tagging | — |
| 15 | Release tracklist | — |
| 16 | Mood & Energy | `mood_enabled` |
| 17 | Lyrics transliterate (AI) | `lyrics_xlit_enabled` **and** `lyrics_translate_enabled` |
| 19 | Optimize artist images | `artist_image_enabled` |
| 20 | Optimize library layout | — |
| 21 | Fix AcoustID pairs | `acoustid_enabled` |
| 22 | Submit fingerprints (AcoustID) | `acoustid_enabled` (opt-in; not in Run All) |
| 23 | Optimize tags | `strip_unknown_tags` |
| 24 | Web ratings | `web_ratings_enabled` |

With a gate off the script is **skipped with the switch named**, not run as a
no-op (`la-musica:mlo/scripts.py:47`).

### 11.2 Run order

```
[11, 3, 14, 15, 2, 1, 13, 17, 8, 24, 5, 19, 6, 7, 9, 12, 16, 10, 23, 20, 21, 4]
```

22 entries; **22 is deliberately absent** (opt-in). `run_all_order` overrides it.

### 11.3 Import chain

`DEFAULT_CHAIN = [sid for sid in run_all_order if sid not in LIBRARY_WIDE_SCRIPTS]`
and `LIBRARY_WIDE_SCRIPTS` is currently empty, so the import chain **equals the
run-all order** (`la-musica:server/imports.py:385`). `import_scripts` replaces it
outright; `import_auto_scripts` off means the chain is not auto-run.

Album movers: `{8, 11, 14, 20}`. A scoped run confines the scan and the fixes to
its targets and stores no library-wide report.

### 11.4 Result record

Per run: `total_scanned, modified_count, unchanged_count, skipped_count,
error_count, total_bytes_added, total_bytes_removed, errors[]`
(`la-musica:mlo/stats.py:34`). Per file: `ok` | `skip` (with a named reason) |
`fail` (with a named reason); a script never raises.

---

## 12. Tools

Installed into `<music>/.mlo/tools/<Name> v<version>[-<host tag>]/`; detected by
running the tool **and** by the marker files the installer wrote
(`la-musica:mlo/fetchdeps.py`). The doctor prints, per tool: expected version,
found version, install kind (download / system package / unsupported here), path,
and the last failure with its log. A tool that installs but cannot run is
reported with its captured stderr and the command line that produced it — never a
bare "error".

---

## 13. Mappings and extensions

| Concern | la-musica | mlo |
|---|---|---|
| config file | `<music>/.mlo/data/config.json` | `mlo.toml`; imports a la-musica `config.json` on first run and preserves unknown keys in `[extra]` |
| index | `tagindex.sqlite` + beets `beets-library.db` | own `index.db` (§3); beets/auth/events are ignored, never converted |
| layout report | `layout_report.json` (§4) | identical |
| trash | flat bin + `.mlo_manifest.json` (§10) | identical (old timestamped layout also read) |
| checks | `grade_check_*` / `grade_include_*` (§2.2) | identical names, labels and defaults |
| presets | Strict / Balanced / Relaxed | identical |
| layout charging | separate panel | **mlo extension**: each finding is charged as `LAYOUT_<KIND>` on its row (Rust brief §9.2) |
| server, web UI, AI, playlists, users, push | la-musica's | out of scope for a single-binary TUI (Rust brief §0) |

### Honest limits

- The AI-backed passes (Mood & Energy, Lyrics transliterate, advisory) report
  *unavailable (model not installed)* until a model/provider is configured.
- `fpcalc`-compatible fingerprinting and `.rar` extraction need the external
  binary; both degrade with a named reason.
- MP4/WAV/AIFF tag writes cover the shared field set; a key with no home there is
  reported as *unplaced*, never invented.
- la-musica's server-only stores, its web/desktop/mobile surfaces and its
  multi-user features are not implemented; the library, config and grading
  contract above is what the two apps share.