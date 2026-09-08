# listings

Upstream: the `listings` package of TeX Live, from the tlnet snapshot at
`https://texlive.info/tlnet-archive/2026/08/01/tlnet/archive/listings.tar.xz`.

Files are byte-for-byte upstream, with their copyright headers intact.
Only the three files the article loads are kept.
The per-language definition files and the documentation driver are not used
here, because the article defines its own language.

TeX Live's catalogue records the package as `lppl1`.
`LICENSE` holds LPPL 1.3c, from `https://www.latex-project.org/lppl/`,
because neither the package nor the TeX Live archive ships the license text.

Vendored rather than fetched: `texlive.info` answers Bazel with a different
byte stream on every request, so a content-addressed pin cannot match it.
`docs/document-build-plan.md` section 3 has the measurements.
