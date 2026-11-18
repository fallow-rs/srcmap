# Scopes decoding cases of tc39/source-map-tests

These files are a copy of `decoding/scopes` in
[tc39/source-map-tests](https://github.com/tc39/source-map-tests), at commit
`bc255a703449b2bc10989772d940df5bf37acf4f`. The suite has the BSD license in
`LICENSE.md`.

Each `<name>.map` file is a source map with `scopes` and `ranges` fields in the
layout of the [ECMA-426 draft specification](https://tc39.es/ecma426/branch/proposal-scopes/).
Each `<name>.map.golden` file is the decoded source map record that the
specification gives for it.

`tests/conformance.rs` decodes each map and compares the original scopes and
the generated ranges with the golden file.

To update the copy, replace the `.map` and `.map.golden` files with the files
of a newer commit, and change the commit in this file.
