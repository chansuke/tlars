# Third-party notices

tlars contains material from the projects below, each under the MIT License.

## TLC (tlaplus/tlaplus)

Source: https://github.com/tlaplus/tlaplus at bc1b506bdcad80bb518bf3776a9161c34ff0fc4c.

`crates/tlars-sany/StandardModules/` is copied from `tla2sany/StandardModules`. `crates/tlars-sany/src/parser/generated.rs` is translated from TLC's Java sources by `xtask jj2rs`, and other generated tables (`crates/tlars-msg/src/catalog.rs`, `crates/tlars-sany/src/error_codes.rs`, `crates/tlars-tool/src/fp64_polys.rs`) are derived from them. The golden files under `crates/*/tests/golden/` and `crates/tlars-conformance/expected/` record TLC's output.

```
MIT License

Copyright (c) 199? HP Corporation
Copyright (c) 2003 Microsoft Corporation
Copyright (c) 2023 Linux Foundation

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

## TLA+ Examples (tlaplus/Examples)

Source: https://github.com/tlaplus/Examples at 334c3983a2f44ac78fce89d1d20aa79ceb3e6987.

The golden files `crates/tlars-sany/tests/golden/parse-examples.txt` and `semantic-examples.txt` contain text from these specifications, and `bench/suite.txt` names them.

```
All these TLA+ examples are licensed under the MIT License.

Copyright (c) 2016: The TLA+ project and other contributors:
https://github.com/tlaplus/Examples/graphs/contributors


Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## tla-ts

Source: https://github.com/aidenybai/tla-ts at 9614c1cabb9b46c9b3a09f0b55cbc11dd485a038.

`crates/tlars-conformance` ports tla-ts's conformance harness (`packages/conformance/scripts/test-file.ts` and `transcript.ts`), and `crates/tlars-conformance/expected/` names the cases of its corpus.

```
MIT License

Copyright (c) 2026 Aiden Bai

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
