# Makefile

Run from the repository root. Build artifacts go to `target/`; packages go to `dist/`.

| Command                                 | Description                                              |
|-----------------------------------------|----------------------------------------------------------|
| `make web`                              | Build the dashboard UI into `server/assets`              |
| `make build`                            | Debug-build `orbien-server` and `orbien`                 |
| `make release`                          | Run `web`, then release-build server and client          |
| `make orbien-server`                    | Run `web`, then release-build the server                 |
| `make orbien`                           | Release-build the client                                 |
| `make desktop-dev`                      | Run the desktop app (dev)                                |
| `make desktop-font-subset`              | Rebuild embedded fonts (i18n + GB2312 L1, ~1.8 MB)       |
| `make desktop-font-subset-tiny`         | Regular-only fonts (~0.9 MB)                             |
| `make desktop-font-subset-full`         | Full SC regional fonts (~16 MB)                          |
| `make desktop-build`                    | Release-build the desktop app                            |
| `make desktop-app` / `make desktop-dmg` | Package macOS `.app` / `.dmg`                            |
| `make desktop-windows`                  | Package Windows portable zip (run on Windows)            |
| `make desktop-deb`                      | Package Linux `.deb` (run on Debian/Ubuntu)              |
| `make package`                          | Run `release`, then copy binaries and `conf/` to `dist/` |
| `make test`                             | Run workspace tests                                      |
| `make fmt`                              | Format Rust code                                         |
| `make clean`                            | Clean `target/` and frontend install/build outputs       |
