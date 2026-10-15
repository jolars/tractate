# Changelog

## [0.2.0](https://github.com/jolars/tractate/compare/v0.1.0...v0.2.0) (2026-10-15)

### Features

- add framed R runtime shim ([`959e9c6`](https://github.com/jolars/tractate/commit/959e9c6f0f2a532cdd10865ef9247c0778d05736))
- discover R and record runner identity ([`84cfc4a`](https://github.com/jolars/tractate/commit/84cfc4a5b20ac88017b023a4e3c85b06cf70a860))
- stabilize semantic identities across revisions ([`eca47f5`](https://github.com/jolars/tractate/commit/eca47f5c8e5cfd5eefde71b29fb56378e7b9b818))
- expose execution states to renderers ([`4222b37`](https://github.com/jolars/tractate/commit/4222b37a3464c6abf2c4a63057483690bfc052a6))
- commit execution artifacts transactionally ([`ad5b4ca`](https://github.com/jolars/tractate/commit/ad5b4cabefe11a72d0a2e740abc8e3b697e4bfcb))
- reconcile revision-safe execution ([`079eb2d`](https://github.com/jolars/tractate/commit/079eb2de62a4a6b3496d8566d01f93562ce7558a))
- add deterministic counting fake runner ([`5fecb3a`](https://github.com/jolars/tractate/commit/5fecb3a5375bac35ff91290aaaa0dfecfe2702d1))
- compute cumulative evaluation state keys ([`374cb0d`](https://github.com/jolars/tractate/commit/374cb0d72984a58aafd63e7ce9a7f87d08e74d5e))
- schedule independent evaluation chains ([`6354474`](https://github.com/jolars/tractate/commit/635447464f020c06766a5585d7246533869343dc))
- lower cells into pure evaluation plans ([`50687a8`](https://github.com/jolars/tractate/commit/50687a8d0d18d22892ae401761d6e848ae915ad8))
- define output and runner lifecycle contracts ([`eecc774`](https://github.com/jolars/tractate/commit/eecc774a929c8a161a4ab32382b8f35f3ae0858d))
- define execution request and outcome contracts ([`55592ad`](https://github.com/jolars/tractate/commit/55592adabadfc68f73812ff497be382050903acb))
- report explicit slide change sets ([`ae70085`](https://github.com/jolars/tractate/commit/ae700852e06da73096d2ecf90cdc6eb5c004a34a))
- reuse semantic nodes and slide fragments ([`c5528b8`](https://github.com/jolars/tractate/commit/c5528b8ed43aa13859932309beebea2c9a9a2690))
- track pure compiler query dependencies ([`6585381`](https://github.com/jolars/tractate/commit/65853813a7d394aa7880bb9d60c6bff790a84d1f))
- add revisioned compiler state ([`372bf1d`](https://github.com/jolars/tractate/commit/372bf1dc6287c0528730429cc7fa8de833d91107))
- support `render --no-execute` ([`e45e3b3`](https://github.com/jolars/tractate/commit/e45e3b3308eec41e55c7c6f4bc94f2e271cd7bc5))
- add static HTML render command ([`b3c0e28`](https://github.com/jolars/tractate/commit/b3c0e28339bcaabf265d580eb650823d7ffd7cd3))
- publish HTML directories atomically ([`3699194`](https://github.com/jolars/tractate/commit/36991943dae03d9fa4e4f661a50ab82218b5938a))
- render Markdown, source code, and math in HTML ([`21d9789`](https://github.com/jolars/tractate/commit/21d9789744c957af988b44c12204969dbec2a21c))
- render Reveal sections with semantic slide IDs ([`6454b81`](https://github.com/jolars/tractate/commit/6454b81284aaeac6131979df7300e31288453935))
- add presentation IR with result slots ([`785c4b3`](https://github.com/jolars/tractate/commit/785c4b38ec89fe6b69f2f59ab3ca5a1c13b8ae4e))
- validate document labels and option declarations ([`2cc9934`](https://github.com/jolars/tractate/commit/2cc9934e2b4b573c88b3aaa60fb9b12a8312c97b))
- distinguish semantic IDs, keys, and artifacts ([`7eb737a`](https://github.com/jolars/tractate/commit/7eb737a0da00013c7dcf7591703f94e90badf191))
- represent presentations and slides ([`b3c0a83`](https://github.com/jolars/tractate/commit/b3c0a837ea02a2fe2ed55e79b3d6b62907c9a7fa))
- represent structured diagnostics ([`95ebe4a`](https://github.com/jolars/tractate/commit/95ebe4a849ef71bef56d4022849f49a488be9af6))
- attach source origins to semantic nodes ([`e515ef9`](https://github.com/jolars/tractate/commit/e515ef9c1566ae3457a7da8097dc8f273c0ab882))
- lower source into a recursive semantic IR ([`a3ff8f0`](https://github.com/jolars/tractate/commit/a3ff8f08f5c31edd3640f5ad653d133e89a963d1))
- define MVP slide counting rules ([`7169118`](https://github.com/jolars/tractate/commit/7169118c7defc44ed45890949045ebc283c16424))
- scaffold project ([`26cb5e5`](https://github.com/jolars/tractate/commit/26cb5e540d0ccc39d0aa098d179cee3d02751a75))

### Bug Fixes

- encode SHA-256 digests explicitly ([`3dc3bf4`](https://github.com/jolars/tractate/commit/3dc3bf433f08baf681fee92ca3d0c11eaadb8a33)), fixes [#5](https://github.com/jolars/tractate/issues/5)
- **ci:** preserve generated changelog and MSRV ([`09446b9`](https://github.com/jolars/tractate/commit/09446b97b83398637f7d4b56c3dbb533d8150d6f))

This file is managed by Versionary.
