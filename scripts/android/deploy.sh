#!/bin/bash
# Installs and drives the APK on the test tablet (`ssh tablet`: rooted
# SM-T720, Android 15). Every file we put on the device lives in
# /data/local/tmp/baba-port/ (or in the app's own directories, which go with
# `pm uninstall`) and is removed by `clean`.
#
# Usage: [APP=probe] scripts/android/deploy.sh <cmd>...
#   install       build (unless NO_BUILD=1) + pm install -r the APK
#   start         (re)start the activity, clearing logcat first
#   import ZIP    push a zip of the game and open it with the app (VIEW
#                 intent, the player's import path); the zip is deleted after
#   log           print our logcat lines (tag babaport/babaprobe + crashes)
#   shot [f]      screencap into $SHOT_DIR (default scratch/android)/<f or shot.png>
#   tap X Y       inject a touch;  key CODE  inject a gamepad key (e.g. BUTTON_A)
#   pad [ARGS]    run the virtual uinput gamepad (padsim) once
#   setprop K V   set a debug.babaport.* switch (see crates/android/src/lib.rs)
#   stop          force-stop the app
#   uninstall     remove the app (and with it its data and imported game files)
#   clean         uninstall + delete /data/local/tmp/baba-port + reset switches
# Commands run in order: deploy.sh install start shot log
set -euo pipefail
source "$(dirname "$0")/../../env.sh"
cd "$BABA_ROOT"
if [ "${APP:-game}" = probe ]; then
    PKG=org.babaport.probe TAG=babaprobe APK=target/dist/android/baba-probe.apk BUILD="--probe"
else
    PKG=org.babaport TAG=babaport APK=target/dist/android/baba-port.apk BUILD=""
fi
ACT=$PKG/android.app.NativeActivity
D=/data/local/tmp/baba-port
SHOT_DIR=${SHOT_DIR:-scratch/android}
mkdir -p "$SHOT_DIR"

while [ $# -gt 0 ]; do
    cmd=$1; shift
    case $cmd in
        install)
            [ "${NO_BUILD:-0}" = 1 ] || scripts/android/build.sh $BUILD
            ssh tablet "mkdir -p $D"
            scp -q "$APK" tablet:$D/app.apk
            ssh tablet "pm install -r $D/app.apk && rm -f $D/app.apk" ;;
        start)
            ssh tablet "input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; am force-stop $PKG; logcat -c; am start -W -n $ACT" ;;
        import)
            z=$1; shift
            ssh tablet "mkdir -p $D && chmod 755 $D"
            scp -q "$z" tablet:$D/import.zip
            ssh tablet "chmod 644 $D/import.zip; input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; am force-stop $PKG; logcat -c; \
                am start -W -n $ACT -a android.intent.action.VIEW -t application/zip -d file://$D/import.zip" ;;
        log)
            ssh tablet "logcat -d -v time -s $TAG:V AndroidRuntime:E DEBUG:F libc:F" ;;
        shot)
            f=${1:-shot.png}; [ $# -gt 0 ] && shift
            ssh tablet "screencap -d 0 -p $D/shot.png 2>/dev/null || screencap -p $D/shot.png"
            scp -q tablet:$D/shot.png "$SHOT_DIR/$f"
            ssh tablet "rm -f $D/shot.png"
            echo "$SHOT_DIR/$f" ;;
        tap)
            ssh tablet "input tap $1 $2"; shift 2 ;;
        key)
            ssh tablet "input gamepad keyevent $1"; shift ;;
        pad)
            P=target/probes/aarch64-linux-android/release/padsim
            [ -f "$P" ] || { echo "build padsim: cargo build --release --target aarch64-linux-android --manifest-path probes/android/Cargo.toml --target-dir target/probes --bin padsim" >&2; exit 1; }
            ssh tablet "mkdir -p $D"
            scp -q "$P" tablet:$D/padsim
            ssh tablet "chmod 755 $D/padsim && $D/padsim ${PAD_ARGS:-}; rm -f $D/padsim" ;;
        setprop)
            ssh tablet "setprop $1 $2"; shift 2 ;;
        stop)
            ssh tablet "am force-stop $PKG" ;;
        uninstall)
            ssh tablet "pm uninstall $PKG || true" ;;
        clean)
            ssh tablet "pm uninstall $PKG >/dev/null 2>&1 || true; rm -rf $D; \
                setprop debug.babaport.nettest ''; setprop debug.babaport.losectx ''" ;;
        *) echo "unknown command $cmd" >&2; exit 2 ;;
    esac
done
