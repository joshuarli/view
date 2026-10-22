# view

A fast native macOS image and video browser, launched from a terminal.

```sh
swift build -c release
./scripts/build-app.sh
./bin/view ~/Pictures
```

`./bin/view` resolves the directory to an absolute path, rejects missing or non-directory roots, and launches `View.app` as a detached native application. With no argument it uses the current directory. Set `VIEW_APP` to point at another `View.app`.

Requires Swift 6.4 and the macOS 27 SDK (Command Line Tools are sufficient).
