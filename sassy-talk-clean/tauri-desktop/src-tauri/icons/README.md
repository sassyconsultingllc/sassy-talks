# Sassy-Talk Icons

## For Tauri (src-tauri/icons/)

Copy these files to your `src-tauri/icons/` folder:

- `icon.ico` - Windows icon
- `32x32.png` - Small icon
- `128x128.png` - Medium icon  
- `128x128@2x.png` - Retina icon (256x256)
- `icon.png` - Large icon (512x512)

For macOS, you'll need to convert to .icns:
```bash
# On macOS:
mkdir icon.iconset
cp 32x32.png icon.iconset/icon_32x32.png
cp 128x128.png icon.iconset/icon_128x128.png
cp 128x128@2x.png icon.iconset/icon_128x128@2x.png
cp icon.png icon.iconset/icon_512x512.png
iconutil -c icns icon.iconset
```

## For Android (res/)

- `icon.png` - Use as `mipmap-xxxhdpi/ic_launcher.png`
- Scale down for other densities

## For Play Store

- `icon.png` (512x512) - Store listing icon
