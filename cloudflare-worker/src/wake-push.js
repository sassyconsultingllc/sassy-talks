// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-WAKEPUSHROUTE
/**
 * wake-push.js — route a presence wake to FCM or APNs by platform.
 *
 * Presence rows carry platform "fcm" (default / Android) or "apns" (iOS).
 * APNs tokens must never be sent to FCM.
 */

import { sendWakePush } from "./fcm.js";
import { sendApnsWakePush } from "./apns.js";

/** Normalize stored / posted platform; unknown values fall back to fcm. */
export function normalizeWakePlatform(platform) {
  return platform === "apns" ? "apns" : "fcm";
}

/**
 * @param {object} env
 * @param {{ token: string, platform?: string }} presence
 * @param {string} roomId
 * @returns {Promise<{ ok: boolean, status: number, error: string|null, stale: boolean }>}
 */
export async function routeWakePush(env, presence, roomId) {
  const token = presence?.token;
  const platform = normalizeWakePlatform(presence?.platform);
  if (platform === "apns") {
    return sendApnsWakePush(env, token, roomId);
  }
  return sendWakePush(env, token, roomId);
}

/** True when at least one wake transport has the secrets it needs. */
export function anyWakeTransportConfigured(env) {
  if (!env) return false;
  if (env.FCM_SERVICE_ACCOUNT_JSON) return true;
  const keys = ["APNS_KEY_ID", "APNS_TEAM_ID", "APNS_PRIVATE_KEY", "APNS_BUNDLE_ID"];
  return keys.every((k) => typeof env[k] === "string" && env[k].trim());
}
