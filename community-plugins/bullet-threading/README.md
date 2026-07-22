# Bullet threading for Tine

An intentionally tiny behavioral port of Peng Xiao's Logseq plugin. Enabling it
activates Tine's constrained `thread-lines` decoration. Tine draws the connectors;
the plugin cannot inject CSS or touch graph data.

Its plugin page offers all-outlines versus active-ancestry display and subtle versus
standard intensity. The public source repository contains the screenshot and full
usage guide.

Build with `cargo build --release`, then run Tine's plugin checker on this directory.
Licensed MIT. AI-primary development, reviewed and published by Martin Koutecký.
