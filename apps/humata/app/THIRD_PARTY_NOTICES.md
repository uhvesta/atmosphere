# Third-party notices

Humata includes or is built with the software below. Each is under its own
license, not Humata's.

## Ghostty

Humata's terminals are drawn by GhosttyKit, Ghostty's library, built from
the source of Ghostty 1.4.0 (<https://ghostty.org>,
<https://github.com/ghostty-org/ghostty>) and linked into the app. Parts of
Humata's terminal view are adapted from Ghostty's macOS app.

MIT License

Copyright (c) 2024 Mitchell Hashimoto, Ghostty contributors

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

### Packages GhosttyKit is built with

Ghostty's build pins these packages in its `build.zig.zon`, and the library
on macOS links code from them. Each one's full license is in its source,
which Ghostty's release tarball and its package list name.

| Package | License |
| --- | --- |
| libxev | MIT |
| zig-objc | MIT |
| uucode | MIT |
| z2d | MPL-2.0 |
| zf | MIT |
| libvaxis | MIT |
| FreeType | FreeType License (FTL) |
| libpng | PNG Reference Library License v2 |
| zlib | zlib |
| Oniguruma | BSD-2-Clause |
| simdutf | Apache-2.0 or MIT |
| Highway | Apache-2.0 or BSD-3-Clause |
| Wuffs | Apache-2.0 or MIT |
| utfcpp | BSL-1.0 |
| glslang | BSD-3-Clause, BSD-2-Clause, MIT and Apache-2.0 |
| SPIRV-Cross | Apache-2.0 or MIT |
| Dear ImGui | MIT |
| JetBrains Mono (the built-in font) | SIL Open Font License 1.1 |
| Nerd Fonts, symbols only | MIT |

Portions of this software are copyright © The FreeType Project
(<https://freetype.org>). All rights reserved.

## boss

The Bazel rule that pins and builds GhosttyKit
(`apps/humata/features/terminal/ghostty/ghostty_kit.bzl`) is modelled on
the one in boss (<https://github.com/spinyfin/mono>). None of boss is in
the app itself.

Copyright the boss authors.

Licensed under the Apache License, Version 2.0 (the "License"); you may not
use this file except in compliance with the License. You may obtain a copy
of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
License for the specific language governing permissions and limitations
under the License.

## Zig

GhosttyKit is compiled with Zig 0.17.0 (<https://ziglang.org>), MIT,
Copyright (c) Zig contributors. The compiler isn't in the app; its runtime
support code compiled into the library is under the same license.
