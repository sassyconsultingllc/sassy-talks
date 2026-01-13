# Sassy-Talk Documentation

## Quick Links

| Document | Description |
|----------|-------------|
| [Privacy Policy](legal/privacy-policy.html) | Data handling practices |
| [Terms of Service](legal/terms-of-service.html) | Usage terms |
| [Support](legal/support.html) | FAQ and troubleshooting |
| [Data Deletion](legal/data-deletion.html) | How to delete your data |

## Build Directories

| Platform | Directory | Status |
|----------|-----------|--------|
| Windows/macOS/Linux | `../tauri-desktop/` | ✅ Ready |
| Android Native | `../android-native/` | ✅ Ready |
| iOS Native | `../ios-native/` | 🚧 Planned |

## Play Store Submission

The `legal/` folder contains everything needed for Google Play:

1. **privacy-policy.html** - Upload to your website, link in Play Console
2. **terms-of-service.html** - Optional but recommended
3. **support.html** - Required support URL for Play Store
4. **data-deletion.html** - Required for account deletion compliance
5. **data-safety.md** - Reference for Play Console Data Safety form
6. **play-store-listing.md** - Copy/paste content for store listing

### Upload to Website

Upload the contents of `legal/` to:
```
https://yourdomain.com/sassy-talk/
├── index.html
├── privacy-policy.html
├── terms-of-service.html
├── support.html
└── data-deletion.html
```

Then in Google Play Console, set:
- Privacy Policy URL: `https://yourdomain.com/sassy-talk/privacy-policy.html`
- Support URL: `https://yourdomain.com/sassy-talk/support.html`

## Technical Documentation

- `DESIGN_DOCUMENT.md` - Architecture overview
- `BUILD.md` - Build instructions
- `SECURITY_FEATURES_COMPLETE.md` - Security implementation
- `WORK_PROFILE_SECURITY.md` - Android work profile handling

## License

© 2025 Sassy Consulting LLC. All rights reserved.
