# IEEEtran

Upstream: the `ieeetran` package of TeX Live, from the tlnet snapshot at
`https://texlive.info/tlnet-archive/2026/08/01/tlnet/archive/ieeetran.tar.xz`
(sha256 `112b81e841a71978ebf0b98f68d6a20542c66e16aa9f8c0fe7fb073b91f8c529`,
89368 bytes).

Files are byte-for-byte upstream, with their copyright headers intact.
Only the two files the article loads are kept.
The package's BibTeX styles and example bibliographies are not used here.

`IEEEtran.cls` states LPPL version 1.3.
TeX Live's own catalogue records the package as `lppl1`.
`LICENSE` holds LPPL 1.3c, from `https://www.latex-project.org/lppl/`,
because neither the package nor the TeX Live archive ships the license text.

Vendored rather than fetched: `texlive.info` answers Bazel with a different
byte stream on every request, so a content-addressed pin cannot match it.
`docs/document-build-plan.md` section 3 has the measurements.
