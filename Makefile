.PHONY: release build test clean

release:
	./scripts/build-app.sh

build:
	swift build -c release --product view

test:
	./scripts/test.sh

clean:
	rm -rf .build View.app
