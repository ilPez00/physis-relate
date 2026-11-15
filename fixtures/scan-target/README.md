# Demo fixture

Ten Rust modules copied unmodified from **physis-core v0.2.0**
(https://github.com/ilPez00/physis-core, Apache-2.0), used as scan input by
`physis-relate demo`.

They are not compiled as part of this crate and are not part of the entry's own
code — they exist so the demo needs nothing beyond a physis-core binary and the
Relate CLI. The Apache-2.0 license applies; see the LICENSE shipped in the
physis-core repository.

`relation.rs` was left out deliberately: the scanner skips files above ~5 KB,
and a fixture that silently drops files would make the demo numbers
unreproducible.
