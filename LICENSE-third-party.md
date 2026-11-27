# Third-party code

The third-party code OpenRage's source tree carries or is ported from, with its origin and the notices it requires.

OpenRage ships no game content. The AES key, the hash table and the AWC key are
extracted from the user's own executable. NG keys, which no retail executable
carries, come from a key snapshot: one the user supplies (`magic.dat` beside the
keys cache, or `OPENRAGE_KEY_SNAPSHOT`), or else CodeWalker's snapshot, which
`or-io` bundles as `crates/io/or-io/data/magic.dat` behind its default
`bundled-keys` feature (D18). That snapshot is masked and encrypted under the AES
key, so it decodes only with the key taken from the user's own executable.
`crates/io/or-io/tests/the_key_snapshot_is_in_the_tree_once.rs` holds the tree
to that one copy. Crate dependencies are not listed here, but for the MP3
decoder and encoder below: `Cargo.lock` names each one, and each carries its own licence.

## Ported from CodeWalker

Ported from [dexyfex/CodeWalker](https://github.com/dexyfex/CodeWalker),
Copyright(c) 2017-2019 dexyfex as its `Notice.txt` states, with the permission
of its authors.

| Rust module | CodeWalker source |
|---|---|
| `crates/io/or-io/src/container/rpf/crypto/aes.rs` | `GTACrypto.cs` (`DecryptAESData`, `EncryptAESData`) |
| `crates/io/or-io/src/container/rpf/crypto/ng.rs` | `GTACrypto.cs` (`DecryptNG`, `DecryptNGRoundA/B`, `EncryptBlock`, `EncryptRoundA`, `EncryptRoundB_LUT`) |
| `crates/io/or-io/src/container/rpf/crypto/ng_encrypt_state.rs` | `GTAKeys.cs` (`RandomGauss::Solve`, `RandomGaussRow`, `LookUpTableGenerator::BuildLUTs2`) |
| `crates/io/or-io/src/container/rpf/crypto/magic.rs` | `GTAKeys.cs` (`UseMagicData`, `JenkHash.GenHash(byte[])`) — decodes a key snapshot, the user's or the bundled one |
| `crates/io/or-io/data/magic.dat` | CodeWalker's key snapshot, bundled under D18 |
| `crates/io/or-io/src/container/rpf/crypto/extractor.rs` | `GTAKeys.cs` (`Generate`, `HashSearch.SearchHashes`) — the AWC key fingerprint is OpenRage's own measurement, not ported |
| `crates/io/or-io/src/container/rpf/crypto/keys.rs` | `GTAKeys.cs`, `GTACrypto.cs` (NG key index derivation, `GTA5NGLUT.LookUp`) |
| `crates/io/or-io/src/container/rpf/crypto/dotnet_random.rs` | .NET Framework `System.Random`, as CodeWalker uses it for the snapshot mask |
| `crates/io/or-io/src/container/rpf/crypto/hashes.rs` | `GTAKeys.cs` (`GTA5KeyHashes`, the SHA-1 fingerprints the extractor matches) |
| `crates/io/or-io/src/container/rpf/crypto/encrypt_cache.rs` | `CryptoIO.WriteLuts` / `ReadNgLuts` (same payload, different framing) |
| `crates/io/or-io/src/container/rpf/crypto/cache.rs` | the keys-cache format, after CodeWalker's key persistence |

The crypto algorithms themselves come from code CodeWalker carries under the
2015 Neodymium MIT notice in its `Notice.txt`. That notice is reproduced at the
end of this file because MIT requires it; it is not CodeWalker's own licence,
and CodeWalker's repository has no top-level licence file.

## Ported from CodeX

Ported from CodeX with the permission of its authors.

| Rust module | CodeX source |
|---|---|
| `crates/io/or-io/src/meta/binary/mod.rs` | `CodeX.Games.GTA5/Files/PsoFile.cs`, `Files/RbfFile.cs`, `RSC7/Rsc7Meta.cs` (container dispatch) |
| `crates/io/or-io/src/meta/binary/pso/` | `CodeX.Games.GTA5/Files/PsoFile.cs` |
| `crates/io/or-io/src/meta/binary/rbf/` | `CodeX.Games.GTA5/Files/RbfFile.cs` |
| `crates/io/or-io/src/meta/binary/rbm/` | `CodeX.Games.GTA5/RSC7/Rsc7Meta.cs` |

## RDR2 archives

| Where | What |
|---|---|
| `crates/io/or-io/data/rdr2_magic.dat` | A key snapshot for RDR2, bundled under the owner's ruling of 2026-10-08 as D18 bundles CodeWalker's. It is CodeX's `CodeX.Games.RDR2/Resources/magic.dat`, masked and encrypted under the game's AES key, so it decodes only with the key from the user's own `RDR2.exe`. |
| `crates/io/or-io/src/container/rpf/crypto/rdr2.rs` | The cipher, ported from `CodeX.Games.RDR2/RPF8/Rpf8Crypto.cs`. |
| `crates/io/or-io/src/container/rpf/lzb16.rs` | OpenRage's own: Oodle's LZB16 read off streams the game's `oo2core` produced and consumed. No code or text of Oodle is in the tree. |
| `oozextract` (vendored in `crates/io/or-kraken/oozextract`, a dependency of `or-kraken`, which `or-io` reaches behind `oodle`) | A pure-Rust port of the Kraken decoder in [ooz](https://github.com/powzix/ooz), which declares MIT in its manifest and ships no licence file; the licence of ooz itself is not stated in its repository, and the question is open (lawyer question 4 covers the bundled snapshot, not this). |

## Ported from bcpack-rs: the BCPack coder of GTA VI texture packs

`crates/io/or-io/src/gta6/bcpack/` decodes the bodies of a GTA VI `.ttpf` pack that are stored
through BCPack, the console's entropy coder over BC blocks. Its entropy layer and its BC1 mode are ported from
[Ahmet-Dedeler/bcpack-rs](https://github.com/Ahmet-Dedeler/bcpack-rs), Copyright (c) 2026 Ahmet
Dedeler, licensed MIT OR Apache-2.0 and taken under MIT, at commit
5bf5cc56ed56f1ababf1f9592300bd182ff90bd8. Each file opens with the header bcpack-rs asks a port to
keep, then its origin.

| Rust file | bcpack-rs source | What |
|---|---|---|
| `crates/io/or-io/src/gta6/bcpack/rans.rs` | rans.rs of its sources | The clocked decoder of the lanes that share one stream. Each lane is, in bcpack-rs, a port of the decoder module of mdu_minirans.v in [microsoft/silicon-rans](https://github.com/microsoft/silicon-rans) (commit 2f6827d), MIT, Copyright (c) Microsoft Corporation. |
| `crates/io/or-io/src/gta6/bcpack/tables.rs` | tables.rs of its sources | The model tables of the coder, which bcpack-rs generates from the same file of silicon-rans. Unchanged but for the checksum constant, which is dropped. |
| `crates/io/or-io/src/gta6/bcpack/bc1.rs` | bc1.rs of its sources | The rebuild of BC1 blocks from the lanes' symbols. |
| `crates/io/or-io/src/gta6/bcpack/bc1_tables.rs` | the two JSON tables of its data folder | The preset endpoints and the quadrant indices of the BC1 mode, which bcpack-rs fitted; the values are unchanged. |
| `crates/io/or-io/src/gta6/bcpack/tests.rs` | encode.rs of its sources | The coder the tests build their streams with; the tests are OpenRage's own. |

`crates/io/or-io/src/gta6/bcpack.rs` is OpenRage's own, and so are
`crates/io/or-io/src/gta6/bcpack/planes.rs`, which rebuilds BC1, BC3, BC4 and BC5 blocks through
tables, and `crates/io/or-io/src/gta6/bcpack/calibrate.rs`, which fits those tables on the user's
machine from the user's own packs. The lane
widths of the modes other than BC1 are facts stated in the documentation of
[ItsNotRexed/Leonida](https://github.com/ItsNotRexed/Leonida)
(GPL-3.0), from which no code and no table is taken. Both MIT notices are reproduced at the end of
this file.

`crates/io/or-io/src/gta6/bcpack/supplied.rs` reads a file of the coder's tables: one a user
supplies, else `crates/io/or-io/data/bcpack-tables.bin`, which the tree holds by the owner's
ruling of 2026-11-27. That file is the one VIctory publishes, byte for byte. No licence is stated
for the tables, whose they are is not known, and OpenRage's licence does not cover them. Nothing
computed from the file's values is in the tree, and every test but the one that reads the tree's
file reads made-up tables. [Marvisak/VIctory](https://github.com/Marvisak/VIctory)
(GPL-3.0-or-later; its asset policy sets the tables themselves outside its licence) publishes
such a file, and [ivall/xcl](https://github.com/ivall/xcl) (GPL-2.0-only) holds the same tables
as generated source. What the reader takes from them is facts about the file's layout:

- the order and lengths of its tables, and the size of an entry of each preset list;
- that its 128 entropy models are the entropy layer's own, which the reader holds a file to;
- that entry `e` of a list is slot `255 - e`;
- that an index table holds places between a block's two ends, not the codes a block stores,
  and a pair table a pair's two places in the two halves of a byte;
- the order of the parts of an entry: BC1's two colours first; BC3's two colours, then its two
  alpha ends; BC4's and BC5's alpha ends first; BC6H's one byte that nothing reads, twelve
  fields, a mode byte counted from 1, then sixteen indices; BC7's word, 24 end values, two depth
  bytes, then sixteen indices;
- that an alpha end, a BC6H field and a BC6H index are each counted from where one with no
  reference starts (128, the top bit alone, half the index's range), a BC6H field left-aligned
  and a BC6H index at the depth its entry's mode gives;
- that a BC7 end is the top byte of a 16-bit value;
- how BC7's table of lane 15's pairs is indexed and what a word of it holds;
- where a BC7 entry's indices start, on which the two projects disagree;
- that a BC7 entry's first word states how many subsets' ends it holds, up to three, and that
  a subset past those reads the first one's.

The tree holds no measurement of these on a pack. What it shows of them is the reader and its
tests, which read a made-up file in this layout
(`crates/io/or-io/src/gta6/bcpack/supplied_tests.rs`), and
`openrage-cli ttpf tables --check` measures a file against a user's own packs: a BC6H or BC7
list read by another layout leaves blocks no encoder writes, and the other tables are held to
the plain mips beside the coded ones. The reader is OpenRage's own code. Two reviews held it
against both projects' sources and found no value of the tables and no translated function. They
found passages where a comment or a construct followed VIctory's, which were rewritten from
OpenRage's own measurement notes and in the terms the tree already had, and a list computed from
a published file's values, which was taken out.

With a tables file kept, a read holds the file's BC1 tables against the built-in BC1 tables of
bcpack-rs and counts the blocks that read a value the two give differently. That comparison is
made on the user's machine, from the user's own file, at each read: no result of it is in the
tree or in a test.

## Derived from Mesa and AMD's instruction description: the gfx10 opcode tables

`crates/tools/or-shader-front/src/rdna/opcodes.rs` lists the opcode numbers and mnemonics of the
instruction set GTA VI's Xbox programs are compiled to. It is generated from the gfx10 column of the
opcode table of ACO, the compiler of [Mesa](https://gitlab.freedesktop.org/mesa/mesa)'s Radeon
driver (aco_opcodes.py of its compiler's sources), Copyright (c) 2018 Valve Corporation, MIT. Thirty rows
ACO does not list, and the corrections to four others, are facts of AMD's machine-readable
description of the RDNA instruction sets
([GPUOpen](https://gpuopen.com/machine-readable-isa)), Copyright (c) 2026 Advanced Micro Devices,
Inc., or its affiliates, MIT, against which every row and every field layout of
`crates/tools/or-shader-front/src/rdna/decode.rs` was checked; that description is not in the tree.
The notice of both is reproduced at the end of this file.

## Carried in the source tree: the DXBC translator

`crates/tools/or-shader-front/vendor/dxbc-spirv/` holds
[dxbc-spirv](https://github.com/doitsujin/dxbc-spirv) at revision `c7f06970`,
Copyright (c) 2025 Philip Rebohle, under the MIT licence its `LICENSE` file states and
which is kept beside it, unmodified. Its `dxbc`, `ir`, `spirv` and `util` source folders
are taken whole, and nothing else of it: its Direct3D 9 front end, its tests, its tools and
its documentation are left out. Two headers of
[SPIRV-Headers](https://github.com/KhronosGroup/SPIRV-Headers), its C++ binding of the SPIR-V grammar and
`GLSL.std.450.h`, sit under `vendor/dxbc-spirv/submodules/spirv_headers/include/`, with that
repository's licence file beside them: the Khronos Group's MIT-style licence, unmodified.
`or-shader-front` compiles the translator, behind its `native` feature, to turn a DXBC shader
into SPIR-V ([R58f](docs/direction/Roadmap.md#r58f-shader-baking-off-windows)).

## Linked: the MP3 decoder

`or-io`'s `awc` feature decodes Enhanced's MP3 `.awc` streams with the
[nanomp3](https://github.com/robbie01/nanomp3) crates, `nanomp3` and
`nanomp3-core`, which are offered under `MIT OR Apache-2.0`. OpenRage takes
them under MIT, whose notice is reproduced at the end of this file. They are a
port of [minimp3](https://github.com/lieff/minimp3), which is CC0 and asks for
no notice. MP3's patents have expired.

## Linked: the MP3 encoder

`or-io`'s `awc` feature encodes the MP3 stream of an Enhanced `.awc` when one is
replaced from a WAV, with the [rusty_mp3](https://crates.io/crates/rusty_mp3)
crate, which is offered under Apache-2.0 and is Copyright 2026 Mata Network. Its
licence is reproduced at the end of this file. It is pure Rust with no
dependencies, and it asks for no NOTICE file: the package carries none.

## Linked: the optimal-parse DEFLATE encoder

`or-io`'s `gta6` feature re-deflates a chunk of a GTA VI patched copy that the
tree's deflater cannot fit under its stored size with the
[zopfli](https://crates.io/crates/zopfli) crate, version 0.8, the zopfli-rs
contributors' Rust translation of Google's Zopfli, offered under Apache-2.0
with the appendix line Copyright 2011 Google Inc. Its terms are the Apache-2.0
text reproduced at the end of this file for rusty_mp3, word for word but for
that line. It is pure Rust, and the package carries no NOTICE file. Only
`crates/io/or-io/src/container/rpf/gta6_patch.rs` calls it.

## Linked: the Opus codec

`or-media` decodes and encodes GTA VI's Opus `.awc` streams with the
[ropus](https://crates.io/crates/ropus) crate, version 0.12.18, a pure-Rust
fixed-point port of the Xiph.Org Opus codec, offered under BSD-3-Clause. Its
licence is reproduced at the end of this file, with the royalty-free patent
licences it points to. Only `crates/io/or-media/src/opus.rs` calls it.

## Bundled: the Inter typeface

The interface's default font is Inter 4.1 by The Inter Project Authors
([rsms/inter](https://github.com/rsms/inter)), offered under the SIL Open Font
License 1.1. `assets/fonts/Inter-Regular.ttf` is the release file with one
change: its character map no longer maps the private-use area (U+E000 to
U+F8FF), where the icon font's glyphs live. Its licence is beside it in
`assets/fonts/Inter-LICENSE.txt`.

## Ported decoders: `or-media`

By the owner's decision of 2026-10-05
([recorded](docs/direction/Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05)),
`crates/io/or-media` holds decoders ported or vendored from open implementations, FFmpeg's
LGPL-2.1-or-later C and NihAV's AGPL-3.0 Rust, so that OpenRage decodes every media format in its
own Rust with no setup. Each ported file opens with a header naming its origin, its upstream path,
the revision it was taken at and its licence, and is listed in the table below with the same four
facts. The `license` field of the crate's `Cargo.toml` states the strictest licence among them. The
licences are not reviewed yet, though the packages of `v0.1.0` link the crate
([policy](docs/guides/Licence_And_Provenance_Policy.md#the-rules)).

| Rust file | Origin | Upstream path | Revision | Licence |
|---|---|---|---|---|
| `crates/io/or-media/src/bink/video.rs` | FFmpeg | `libavcodec/bink.c` | n7.0, commit 083443d67cb159ce469e5d902346b8d0c2cd1c93 | LGPL-2.1-or-later |
| `crates/io/or-media/src/bink/dsp.rs` | FFmpeg | `libavcodec/binkdsp.c` | n7.0, commit 083443d67cb159ce469e5d902346b8d0c2cd1c93 | LGPL-2.1-or-later |
| `crates/io/or-media/src/bink/tables.rs` | FFmpeg | libavcodec, the file binkdata.h, generated by script | n7.0, commit 083443d67cb159ce469e5d902346b8d0c2cd1c93 | LGPL-2.1-or-later |
| `crates/io/or-media/src/bink/audio.rs` | FFmpeg | `libavcodec/binkaudio.c` and `libavcodec/wma_freqs.c` | n7.0, commit 083443d67cb159ce469e5d902346b8d0c2cd1c93 | LGPL-2.1-or-later |
| `crates/io/or-media/src/bik.rs` | FFmpeg | `libavformat/bink.c` | n7.0, commit 083443d67cb159ce469e5d902346b8d0c2cd1c93 | LGPL-2.1-or-later |
| `crates/io/or-media/src/bink2/video.rs` | NihAV | `nihav-rad/src/codecs/bink2.rs` (`Bink2Decoder`, its frame decoder and the readers it calls) | commit 13f31633cf325b0036d697af7bf05c611b4b8967, 2026-09-28 | AGPL-3.0-only |
| `crates/io/or-media/src/bink2/dsp.rs` | NihAV | `nihav-rad/src/codecs/bink2.rs` (`bink2_idct`, `Bink2DSP`) | commit 13f31633cf325b0036d697af7bf05c611b4b8967, 2026-09-28 | AGPL-3.0-only |
| `crates/io/or-media/src/bink2/tables.rs` | NihAV | `nihav-rad/src/codecs/bink2.rs` (the tables after its tests) | commit 13f31633cf325b0036d697af7bf05c611b4b8967, 2026-09-28 | AGPL-3.0-only |
| `crates/io/or-media/src/bink2/filter.rs` | RAD Game Tools' `bink2w64.dll`, the Enhanced install's | the functions at 0x18001d990 and 0x18001d660 (the pass after each 32×32 block), 0x18000d690 and 0x18000d8a0 with the table at 0x180041200 (the edges), 0x18000c7a0, 0x18000cc40 and 0x18000d0e0 (a block that codes nothing), 0x180014870, 0x180014910 and 0x18000e0f0 (the residue of an inter block) | version 2020.07, image base 0x180000000 | proprietary: derived by reading for interoperability, no code or bytes copied |
| `crates/io/or-media/src/bink2/video.rs`, the clamp `Reach` and the block records | RAD Game Tools' `bink2w64.dll`, the Enhanced install's | the functions at 0x18001d350 (where a 16×16 square may read) and 0x180011ae0 (the record of an 8×8 block) | version 2020.07, image base 0x180000000 | proprietary: derived by reading for interoperability, no code or bytes copied |
| `crates/io/or-media/src/bink/mod.rs` | OpenRage's own | none: it declares the modules of the Bink 1 port, which are listed above | none | OpenRage's own (MIT); the crate's `license` states the strictest licence of its files |

FFmpeg n7.0 is the tag of FFmpeg 7.0, and the commit is the one the tag names. The Bink 1 decoders are
port-for-port: the same reads in the same order and the same integer arithmetic, so a picture matches
FFmpeg's bit for bit. Three more files of the crate follow FFmpeg's contract and take no code from
it, and are OpenRage's own: `bits.rs`, a bit reader that takes the lowest bit first, `transform.rs`,
the DCT-III and inverse real DFT that Bink audio decodes with, and `yuv.rs`, the conversion to RGBA.
XMA, Xbox 360's audio codec in the `.awc` files of Legacy's Vinewood casino pack, is
a port of FFmpeg's WMA Pro decoder: xma_decode_packet and `decode_packet` in `wmaprodec.c`
read the packets, and the frame decode follows `decode_frame` and what it calls. Each file below
carries FFmpeg's copyright lines in its header and is LGPL-2.1-or-later; the crate's `license` field
states the strictest licence among all its files, AGPL-3.0-only.

| Rust file | Origin | Upstream path | Revision | Licence |
|---|---|---|---|---|
| `crates/io/or-media/src/xma/decoder.rs` | FFmpeg | `libavcodec/wmaprodec.c`, with ff_wma_run_level_decode and ff_wma_get_large_val from `libavcodec/wma.c` and vector_fmul_window from `libavutil/float_dsp.c` | tag `n7.0`, commit `083443d67cb159ce469e5d902346b8d0c2cd1c93` | LGPL-2.1-or-later |
| `crates/io/or-media/src/xma/tables.rs` | FFmpeg | `libavcodec/wmaprodata`, the C header of that name, transcribed mechanically | tag `n7.0`, commit `083443d67cb159ce469e5d902346b8d0c2cd1c93` | LGPL-2.1-or-later |
| `crates/io/or-media/src/xma/vlc.rs` | FFmpeg | `libavcodec/vlc.c`, the code assignment of ff_vlc_init_from_lengths | tag `n7.0`, commit `083443d67cb159ce469e5d902346b8d0c2cd1c93` | LGPL-2.1-or-later |
| `crates/io/or-media/src/xma/mod.rs` | FFmpeg | `libavcodec/wmaprodec.c`, the output range of xma_decode_packet | tag `n7.0`, commit `083443d67cb159ce469e5d902346b8d0c2cd1c93` | LGPL-2.1-or-later |
| `crates/io/or-media/src/xma/mdct.rs` | FFmpeg, in its test only | `libavutil/tx_template.c`, the reference ff_tx_mdct_naive_inv; the transform is OpenRage's own | tag `n7.0`, commit `083443d67cb159ce469e5d902346b8d0c2cd1c93` | LGPL-2.1-or-later |
| `crates/io/or-media/src/xma/bits.rs` | OpenRage's own | none: a most-significant-bit-first reader and the reservoir a frame spanning packets is assembled in, whose operations are named for FFmpeg's (`get_bits`, `show_bits`, `get_sbits`, `get_bitsz`, get_bits_left and ff_copy_bits) and take no code from it | none; the names are those of tag `n7.0` | OpenRage's own (MIT); the crate's `license` states the strictest licence of its files |

FFmpeg 7.0 is also run as a program, never linked or bundled, by three tests that skip without one:
`crates/title/or-title/tests/every_shipped_bink1_matches_ffmpeg.rs` and
`crates/io/or-media/tests/bink_audio_matches_ffmpeg.rs` hold the Bink decoders to its output, and
`crates/io/or-media/tests/xma_matches_ffmpeg.rs`, install-gated, holds the XMA decode to its `xma2`
decoder.

NihAV is <https://git.nihav.org/?p=nihav.git>, by Kostya Shishkov. Its `COPYING` is the GNU Affero
General Public License version 3, and its README states no later version, so the three Bink 2 files
are AGPL-3.0-only and the crate's `license` field says so; the README adds that parts of the project
can be relicensed to LGPLv2 on request, which the licence review may use. The Bink 2
decoder is port-for-port too, except in what the shipped movies fix against the `bink2w64.dll` of
the install that ships them: the inverse transform takes the columns of a block first, an inter
block's DC takes the same 32 of rounding an intra block's does, the second pass of a chroma vector
in both directions rounds to nearest, and a frame in the coding of `KB2f` and before, which no
shipped movie uses, is refused.

NihAV applies no in-loop filter, and reads a motion vector where it points. What the DLL does
instead is in the two rows above that name it: RAD Game Tools' `bink2w64.dll` is proprietary, and
`or-media` derives from it, for interoperability, the filter a frame states a strength for, the
filter of an inter block's residue, and the clamp that keeps a 16×16 square on the picture, from its
disassembly and decompilation, with the owner's decision of 2026-10-06
([recorded](docs/direction/Project_Context.md#the-bink-2-in-loop-filter-is-derived-from-rads-dll-2026-10-06)).
The code is Rust of OpenRage's own, and the constants and tables it holds are the DLL's. The files
stay in `or-media`, await R66h's licence review, and each opens with a header
naming the DLL file, its version and the functions' addresses. The DLL itself is never copied; a
Windows test loads the one beside the executable of the user's own install, and the Legacy
install's, version 1.994a, gives the same pictures.

---

## Notice: the 2015 Neodymium MIT text, as embedded in CodeWalker's `Notice.txt`

```
Copyright(c) 2015 Neodymium

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## Notice: the MIT text of nanomp3 and nanomp3-core

```
Copyright (c) 2025 Robert B. Langer

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## Notice: the Apache-2.0 text of rusty_mp3

```
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or Derivative
          Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and do
          not modify the License. You may add Your own attribution notices
          within Derivative Works that You distribute, alongside or as an
          addendum to the NOTICE text from the Work, provided that such
          additional attribution notices cannot be construed as modifying
          the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright 2026 Mata Network

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```

## Notice: the BSD-3-Clause text of ropus

```
Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic,
                    Jean-Marc Valin, Timothy B. Terriberry,
                    CSIRO, Gregory Maxwell, Mark Borgerding,
                    Erik de Castro Lopo, Mozilla, Amazon

Copyright (c) 2026 Martin Davidson (Rust port additions)

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of Internet Society, IETF or IETF Trust, nor the
names of specific contributors, may be used to endorse or promote
products derived from this software without specific prior written
permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Opus is subject to the royalty-free patent licenses which are
specified at:

Xiph.Org Foundation:
https://datatracker.ietf.org/ipr/1524/

Microsoft Corporation:
https://datatracker.ietf.org/ipr/1914/

Broadcom Corporation:
https://datatracker.ietf.org/ipr/1526/
```

## Notice: the MIT text of bcpack-rs

```
MIT License

Copyright (c) 2026 Ahmet Dedeler

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Notice: the MIT text of silicon-rans, as bcpack-rs reproduces it

```
MIT License

Copyright (c) Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE
```

## Notice: the MIT text of Mesa's ACO opcode table and of AMD's instruction description

```
Copyright (c) 2018 Valve Corporation
Copyright (c) 2026 Advanced Micro Devices, Inc., or its affiliates.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
