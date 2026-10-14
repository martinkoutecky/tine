#!/bin/bash
# Install the APK, push the demo graph (journal files dated to the device's today/yesterday).
set -eu
adb shell getprop ro.product.cpu.abilist
adb shell getprop ro.build.version.release
adb install -r tine.apk
DEV=$(adb shell date +%Y_%m_%d | tr -d '\r')
NOW=$(adb shell date +%s | tr -d '\r')
DEVY=$(adb shell date -d @$((NOW - 86400)) +%Y_%m_%d | tr -d '\r')
echo "device dates $DEV $DEVY"
cd "Tine demo/journals"
if [ "$DEV" != 2026_10_14 ]; then mv 2026_10_14.md "$DEV.md"; fi
if [ "$DEVY" != 2026_10_13 ]; then mv 2026_10_13.md "$DEVY.md"; fi
cd ../..
adb shell mkdir -p /sdcard/Documents
adb push "Tine demo" /sdcard/Documents/
adb shell 'ls -R "/sdcard/Documents/Tine demo"'
