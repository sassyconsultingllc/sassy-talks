# Auto-Connect & UX Simplification Design

**Date:** 2026-03-29
**Status:** Approved

## Problem

1. DevicePickerScreen forces users to manually choose WiFi Multicast vs Cellular Relay - the app should detect this automatically
2. QR code at 280dp pushes the "Continue" button off-screen on smaller phones
3. Connection flow has unnecessary friction

## Solution

### 1. Remove DevicePickerScreen

**Current flow:** Auth (QR) -> DevicePicker (manual choice) -> Main
**New flow:** Auth (QR) -> Main (auto-connect in background)

DevicePickerScreen is eliminated. After QR auth, navigate directly to MainScreen. PTT is disabled until connection completes.

### 2. AutoConnectManager

New class that handles smart transport selection:

**State flow:** `DETECTING -> TRYING_WIFI -> TRYING_CELLULAR -> CONNECTED | FAILED`

**Logic:**
1. Query `ConnectivityManager` for active network capabilities
2. If `NET_CAPABILITY_WIFI` present:
   - Acquire multicast lock
   - Call `connectWifiMulticast()`
   - Wait 3 seconds for peer beacon response
   - If peer found -> CONNECTED (wifi)
   - If no peer -> fall through to cellular
3. Set room from session_id + connect cellular WebSocket
   - Wait 5 seconds for WebSocket handshake
   - If connected -> CONNECTED (cellular)
   - If failed -> FAILED
4. On FAILED: emit error state for UI retry

### 3. MainScreen Connection States

MainScreen observes AutoConnectManager state:
- `DETECTING` -> "Detecting network..." (PTT disabled, gray)
- `TRYING_WIFI` -> "Trying WiFi..." (PTT disabled)
- `TRYING_CELLULAR` -> "Connecting via relay..." (PTT disabled)
- `CONNECTED` -> Normal PTT, status shows transport name
- `FAILED` -> "Connection failed" + Retry button (PTT disabled)

Remove back-navigation to DevicePickerScreen.

### 4. QR Code Sizing

- Reduce QR display from 280dp to 200dp
- Reduce card padding from 16dp to 12dp
- Reduce spacer below QR from 20dp to 12dp
- "Continue" button visible without scrolling on 5" screens

### Files to Modify

1. **`QRAuthScreen.kt`** - Shrink QR code and spacing
2. **`DevicePickerScreen.kt`** - Delete (or gut and redirect)
3. **`AppNavigation.kt`** - Remove DevicePicker from nav, wire Auth -> Main with auto-connect
4. **`MainScreen.kt`** - Add connection state UI, disable PTT until connected
5. **New: `AutoConnectManager.kt`** - Smart transport detection and connection logic

### Files NOT Changed

- Rust transport layer (already supports both protocols)
- SassyTalkNative JNI bridge (already has all needed methods)
- CellularWebSocketClient (already handles WebSocket lifecycle)
- QR generation logic (only display size changes)
