icon:: ⌨️

- # Command line
	- The installed desktop program also has a small command-line interface. Run `tine --help` for the current command list and `tine --version` (or `tine version`) for the exact build version.
- ## Open Tine
	- `tine open GRAPH` opens a graph. The older shorthand `tine GRAPH` remains supported. If Tine is already running, the graph opens in a new window of that instance.
	- `tine capture` opens Quick Capture. The older `tine --capture` spelling remains supported, so existing desktop shortcuts keep working.
	- `tine --debug` turns on the diagnostic log for that run, exactly like `TINE_DEBUG=1`.
- ## Publish a graph
	- `tine export static GRAPH --output PARENT` writes the static HTML publication. `tine export live GRAPH --output PARENT` writes the read-only Tine app and keeps the static HTML site as its no-JavaScript and `file://` fallback.
	- `PARENT` must be an existing folder outside the graph, given as an absolute path. Tine creates one new child folder in it, named after the graph (or after `--name`), and refuses to replace an existing folder of that name.
	- Publication follows the graph's existing privacy settings: only pages with `public:: true` are included, unless `:publishing/all-pages-public?` is enabled. Add `--all-pages` to override that selection for one export.
	- Example: `tine export live ~/notes --all-pages --output ~/Sites --name "My notes"`
- ## Check a graph
	- `tine doctor GRAPH` reads and parses the graph, reports how many pages and journals it found, and does not change the graph.
- ## Full reference
	- Every command and subcommand has `--help`, for example `tine export live --help`. Linux `.deb` and `.rpm` packages also install the same generated reference for `man tine`.
