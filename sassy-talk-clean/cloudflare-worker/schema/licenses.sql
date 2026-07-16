-- Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
-- Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-- CodeMark: SCLLC1-sassytalkie-KAIVQSH256EU
-- License registry for the direct-distribution (website APK) build.
-- Apply: wrangler d1 execute sassytalkie-licenses --remote --file=schema/licenses.sql
--
-- Raw license keys are NEVER stored. key_hash = HMAC-SHA256(LICENSE_SALT, "lic."||key),
-- device_hash = HMAC-SHA256(LICENSE_SALT, "dev."||device_id). See src/license.js.

CREATE TABLE IF NOT EXISTS licenses (
  key_hash    TEXT PRIMARY KEY,
  email       TEXT,
  note        TEXT,
  status      TEXT NOT NULL DEFAULT 'active',   -- active | revoked
  max_devices INTEGER NOT NULL DEFAULT 3,
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS activations (
  key_hash    TEXT NOT NULL REFERENCES licenses(key_hash),
  device_hash TEXT NOT NULL,
  device_name TEXT,
  app_version TEXT,
  first_seen  TEXT NOT NULL DEFAULT (datetime('now')),
  last_seen   TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (key_hash, device_hash)
);

CREATE INDEX IF NOT EXISTS idx_activations_key ON activations(key_hash);

-- Promo codes: one shared code, many redemptions, device-bound like licenses.
-- code_hash = HMAC-SHA256(LICENSE_SALT, "promo."||UPPERCASE(code)).
CREATE TABLE IF NOT EXISTS promo_codes (
  code_hash       TEXT PRIMARY KEY,
  note            TEXT,
  status          TEXT NOT NULL DEFAULT 'active',   -- active | revoked
  max_redemptions INTEGER NOT NULL DEFAULT 100,
  expires_at      TEXT,                             -- ISO8601, NULL = never
  created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS promo_redemptions (
  code_hash   TEXT NOT NULL REFERENCES promo_codes(code_hash),
  device_hash TEXT NOT NULL,
  device_name TEXT,
  app_version TEXT,
  redeemed_at TEXT NOT NULL DEFAULT (datetime('now')),
  last_seen   TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (code_hash, device_hash)
);

CREATE INDEX IF NOT EXISTS idx_promo_redemptions_code ON promo_redemptions(code_hash);
