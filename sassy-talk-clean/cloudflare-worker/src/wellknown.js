// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-ZCCZ3DH45KOG
/**
 * wellknown.js — Digital Asset Links (Android) + Apple App Site Association
 * (iOS/macOS) for relay.sassyconsultingllc.com.
 *
 * These let the OS verify that the SassyTalk app owns the /v/ path, so a tapped
 * invite link
 *   https://relay.sassyconsultingllc.com/v/<id>#<key>
 * opens the app DIRECTLY (verified Android App Link / Apple Universal Link)
 * instead of falling through to the /v/ browser landing page (viewer.js).
 *
 * The AndroidManifest declares android:autoVerify="true" on the /v/ filter and
 * its comment promises this file is "published on the worker" — this route is
 * what makes that true.
 *
 * Identifiers are env-sourced, never hardcoded, because they're deploy-specific
 * and (for Android) come from Play, not this repo:
 *   - APPLE_APP_ID        full "<TeamID>.com.sassyconsulting.sassytalkie".
 *                         Or set APPLE_TEAM_ID alone and the bundle id below is
 *                         appended.
 *   - ANDROID_CERT_SHA256 one or more SHA-256 signing-cert fingerprints
 *                         (colon-hex), separated by commas/space/newlines. Use
 *                         the **Play App Signing** cert (Play Console → Test and
 *                         release → App integrity), NOT the local upload
 *                         keystore — Google re-signs the app. Add the upload key
 *                         too if you also sideload that build.
 *
 * Served as application/json with no redirects (Apple fetches AASA without
 * following 3xx). The AASA path deliberately has no file extension.
 */

const ANDROID_PACKAGE = "com.sassyconsulting.sassytalkie";
const APPLE_BUNDLE_ID = "com.sassyconsulting.sassytalkie";

export function handleWellKnownRoute(request, env, url) {
  const path = url.pathname;
  const isAasa = path === "/.well-known/apple-app-site-association";
  const isAssetlinks = path === "/.well-known/assetlinks.json";
  if (!isAasa && !isAssetlinks) return null;
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("Method not allowed", { status: 405 });
  }

  const { doc, configured } = isAasa ? appleAasa(env) : androidAssetlinks(env);
  return new Response(JSON.stringify(doc), {
    status: 200,
    headers: {
      "Content-Type": "application/json",
      // Verifiers re-fetch periodically; an hour lets a team-id/fingerprint fix
      // propagate without a manual cache bust.
      "Cache-Control": "public, max-age=3600",
      // Quick diagnosis: `curl -I` shows whether the env identifiers are set,
      // so an unverified link doesn't look like a worker bug.
      "X-Wellknown-Configured": String(configured),
    },
  });
}

function resolveAppleAppId(env) {
  const explicit = env && env.APPLE_APP_ID && env.APPLE_APP_ID.trim();
  if (explicit) return explicit;
  const team = env && env.APPLE_TEAM_ID && env.APPLE_TEAM_ID.trim();
  return team ? `${team}.${APPLE_BUNDLE_ID}` : "";
}

function appleAasa(env) {
  const appID = resolveAppleAppId(env);
  // Legacy appID+paths form — honored on every iOS/macOS version. Until an app
  // id is configured we serve a valid-but-empty document (verification simply
  // won't pass), rather than 404, so the file is always present for Apple.
  const details = appID ? [{ appID, paths: ["/v/*"] }] : [];
  return {
    doc: { applinks: { apps: [], details } },
    configured: details.length > 0,
  };
}

function androidAssetlinks(env) {
  const raw = (env && env.ANDROID_CERT_SHA256) || "";
  const fingerprints = raw
    .split(/[\s,]+/)
    .map((s) => s.trim().toUpperCase())
    .filter(Boolean);
  const doc = fingerprints.length
    ? [
        {
          relation: ["delegate_permission/common.handle_all_urls"],
          target: {
            namespace: "android_app",
            package_name: ANDROID_PACKAGE,
            sha256_cert_fingerprints: fingerprints,
          },
        },
      ]
    : [];
  return { doc, configured: fingerprints.length > 0 };
}
