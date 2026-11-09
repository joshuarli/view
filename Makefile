.PHONY: build test package install bench

APP := View.app
INSTALL_DIR ?= /Applications

build:
	cargo build --profile dist --bin app --features app

test:
	cargo test --profile dist

package:
	sh scripts/package-app.sh

install: package
	mkdir -p $(INSTALL_DIR)
	cp -R $(APP) $(INSTALL_DIR)/
	echo "installed $(INSTALL_DIR)/$(APP)"

bench:
	sh scripts/bench.sh $(BENCH) $(BENCH_ARGS)
