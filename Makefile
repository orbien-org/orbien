ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
export CARGO_TARGET_DIR := $(ROOT)/target

WEB_DIR := server-ui

web:
	cd $(WEB_DIR) && npm install && npm run build
	@echo "dashboard assets -> server/assets"

build:
	cargo build -p orbien-server -p orbien-client

release: web
	cargo build --release -p orbien-server -p orbien-client
	@echo ""
	@echo "artifacts:"
	@ls -lh target/release/orbien-server target/release/orbien

orbien-server: web
	cargo build --release -p orbien-server
	@echo ""
	@echo "artifact:"
	@ls -lh target/release/orbien-server

orbien:
	cargo build --release -p orbien-client

desktop-dev:
	cargo run -p orbien-desktop

desktop-font-subset:
	python3 scripts/gen-desktop-font-subset.py

desktop-font-subset-tiny:
	python3 scripts/gen-desktop-font-subset.py --regular-only

desktop-font-subset-full:
	python3 scripts/gen-desktop-font-subset.py --full

desktop-build:
	cargo build --release -p orbien-desktop

desktop-app desktop-dmg: desktop-build
	chmod +x scripts/pack-desktop-macos.sh
	./scripts/pack-desktop-macos.sh
	@echo "app -> dist/Orbien Desktop.app"
	@echo "dmg -> dist/orbien-desktop_*_darwin_*.dmg"

desktop-windows: desktop-build
	chmod +x scripts/pack-desktop-windows.sh
	./scripts/pack-desktop-windows.sh
	@echo "exe/zip -> dist/orbien-desktop_*_windows_*"

desktop-deb: desktop-build
	chmod +x scripts/pack-desktop-linux-deb.sh
	./scripts/pack-desktop-linux-deb.sh
	@echo "deb -> dist/orbien-desktop_*_linux_*.deb"

test:
	cargo test --workspace

docker-context:
	chmod +x scripts/prepare-docker-context.sh scripts/stage-docker-slice.sh
	./scripts/prepare-docker-context.sh --upx

docker-client: docker-context
	docker build --pull=false -f docker/Dockerfile.client -t orbien:local .

docker-server: docker-context
	docker build --pull=false -f docker/Dockerfile.server -t orbien-server:local .

fmt:
	cargo fmt --all

clean:
	cargo clean
	rm -rf $(WEB_DIR)/node_modules $(WEB_DIR)/dist

package: release
	mkdir -p dist
	cp target/release/orbien-server target/release/orbien dist/
	cp -R conf dist/
	@echo "packaged -> dist/"
	@ls -lh dist/
