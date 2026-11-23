# Changelog

## [0.2.0](https://github.com/meiazero/alpheus/compare/v0.1.2...v0.2.0) (2026-11-23)


### Added

* **desktop:** add a GPUI window that lists disks in the fix-storage style ([31257fe](https://github.com/meiazero/alpheus/commit/31257fecc831bcba0aef4d627a678944468beb58))
* **desktop:** add a gpui window that lists mounted disks ([ee4391a](https://github.com/meiazero/alpheus/commit/ee4391a7365ec3aa92be36f08e9e4ae04fd6f473))
* **desktop:** draw the disk meter as a dot grid and fit the window to its disks ([7e1b2d9](https://github.com/meiazero/alpheus/commit/7e1b2d9247ed64ec5e2694c0b5b186bafd09251f))
* **desktop:** lead with the home disk and open at 720p ([1b26551](https://github.com/meiazero/alpheus/commit/1b26551ad549c5972b3811191e5f58edd56c2fc1))
* **desktop:** make the disk meter the first thing you see ([566088e](https://github.com/meiazero/alpheus/commit/566088e5dcbfad8965bb82f34842d8e79df054bf))
* **desktop:** open at 720p with the home disk first and the rest as cards ([5f20d46](https://github.com/meiazero/alpheus/commit/5f20d46873b74974c431e63e9c6c852bd7e3a772))

## [0.1.2](https://github.com/meiazero/alpheus/compare/v0.1.1...v0.1.2) (2026-11-23)


### Fixed

* **tui:** return to retained explorer without rescanning ([6038992](https://github.com/meiazero/alpheus/commit/6038992dcb7aaf7b9b19aad8bc9483b0df06e625))
* **tui:** return to the explorer without rescanning the disk ([3b535c4](https://github.com/meiazero/alpheus/commit/3b535c4dbd573746a91a26f6cffd4faf91a06c6c))

## [0.1.1](https://github.com/meiazero/alpheus/compare/v0.1.0...v0.1.1) (2026-11-16)


### Fixed

* **clean:** block candidates whose contents could not be verified ([a92ab5a](https://github.com/meiazero/alpheus/commit/a92ab5a4c0961bfe360a6d79e9df25e76e7ee6e2))
* **clean:** never offer a candidate whose contents could not be verified ([3967a93](https://github.com/meiazero/alpheus/commit/3967a93fc0e44d21ea8ac82cdbea76341f0eff7f))
* **tui:** keep bar colors on the selected row and hide entries you cannot remove ([2107691](https://github.com/meiazero/alpheus/commit/2107691c68eedb68aa0dda8d14146185d81abe2a))
* **tui:** keep usage bars visible on the selected row and hide entries you cannot remove ([5dbb38d](https://github.com/meiazero/alpheus/commit/5dbb38dadb61df3ad01bcef4793f40fe88a84ea5))

## 0.1.0 (2026-11-16)


### Added

* add --version flag ([fe6228e](https://github.com/meiazero/alpheus/commit/fe6228ee95ad384800387aa9de690a8c3302898d))
* add disk usage widget, explorer tui and safe cleanup ([953670f](https://github.com/meiazero/alpheus/commit/953670f17de3d7834a29cc02292a688b15c4db1c))
* **install:** add curl installer that verifies the release checksum ([6ef1528](https://github.com/meiazero/alpheus/commit/6ef15287f2d0fa9ba2b4876f84de500c081f6f06))


### Fixed

* sort by key to satisfy clippy unnecessary_sort_by ([aa571f8](https://github.com/meiazero/alpheus/commit/aa571f8cedf7661092cec5fc62b4e42a085c3a6f))


### Documentation

* add readme and mit license ([d495760](https://github.com/meiazero/alpheus/commit/d495760c0bfe3df369595171eecb09e2c131428e))
* **readme:** document curl install and release flow ([f54ce17](https://github.com/meiazero/alpheus/commit/f54ce1708df0da01f2771bf4aa07c89e9ed0e2e6))
