# Demo fixture

Ten Rust modules copied unmodified from **physis-core v0.2.0**
(https://github.com/ilPez00/physis-core, Apache-2.0), plus this README, used as
scan input by `physis-relate demo`.

They are not compiled as part of this crate and are not part of the entry's own
code — they exist so the demo needs nothing beyond a physis-core binary and the
Relate CLI. The Apache-2.0 license applies; see the LICENSE shipped in the
physis-core repository.

`relation.rs` (5748 bytes) is included for completeness but physis-core's scanner
skips files over 5 KB, so it is not registered. This README is text and is
registered, which is why a run reports **10 nodes from 11 files**: nine of the
ten `.rs` modules plus this README. That is a property of the scanner, not of
this fixture, and it is why the demo numbers are what they are.

Do not commit a `.physis-next/` directory into this tree. A `physis-next observe`
run writes one, and it then becomes scan input, so the demo stops touching a
real source file. It is git-ignored at the repository root.
