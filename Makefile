.PHONY: release build test bench clean

release:
	./scripts/build-app.sh

build:
	swift build -c release --product view

test:
	./scripts/test.sh

bench:
	swift run -c release ViewBenchmarks -- $(ARGS)

clean:
	rm -rf .build View.app
