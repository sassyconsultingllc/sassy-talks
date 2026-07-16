@echo off
:: SassyTalkie PTT/BT Debug Log
:: Shows only BT/PTT-related logcat output for fast debugging.
:: Run in a separate terminal while testing.

echo === SassyTalkie PTT/BT Debug Log ===
echo Filtering: SassyTalk-JNI, SassyTalkNative, BluetoothTransport
echo Press Ctrl+C to stop
echo.

adb logcat -c
adb logcat -v time -s "SassyTalk-JNI:*" "SassyTalkNative:*" "BluetoothTransport:*" "sassy-tx:*" "sassy-rx:*" "bt-accept:*" "bt-tx-pump:*" "bt-rx-*:*" "bt-dead-peer:*"
