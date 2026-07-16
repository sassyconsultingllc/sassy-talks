// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-5IUDZKREFOER
/**
 * viewer.js — Browser landing page for /v/<id> session-invite links.
 *
 * An invite URL looks like:
 *   https://relay.sassyconsultingllc.com/v/<id>#<base64url-key>
 *
 * On Android the app claims this path as a verified App Link (see
 * AndroidManifest pathPrefix "/v/"), so a tap opens SassyTalk directly and this
 * page is never seen. This route is the FALLBACK for everyone else: a recipient
 * without the app installed (iOS, desktop, or any browser, or an Android device
 * where link handling is disabled) would otherwise hit a bare 404. Instead they
 * get a chooser: "Open in SassyTalk" (if installed) + install links.
 *
 * Privacy invariant — this route NEVER touches the share blob:
 *   - The decryption key lives only in the URL #fragment, which browsers never
 *     transmit to the server. So the worker cannot see it, and the page resolves
 *     the full deep link entirely client-side from `location.hash`.
 *   - We do NOT fetch GET /share/<id> here and we do NOT burn the one-time blob.
 *     This is a static chooser; the real fetch+decrypt only happens inside the
 *     app after it opens the link.
 *
 * Because the page is identical for every id (the id/key are read client-side),
 * a single static document is served for all of them and is freely cacheable.
 */

// Mirrors share.js ID_RE: base64url, 16–64 chars. An id that can't be a real
// share id gets a 404 so this route stays invisible to scanners.
const ID_RE = /^[A-Za-z0-9_-]{16,64}$/;

const APK_URL = "https://relay.sassyconsultingllc.com/dl/apk";
const PLAY_URL =
  "https://play.google.com/store/apps/details?id=com.sassyconsulting.sassytalkie";

/**
 * Handle GET /v/<id>. Returns a Response for the landing page, or null so the
 * caller falls through to its normal routing (other methods / non-/v/ paths).
 */
export function handleViewerRoute(request, url) {
  const path = url.pathname;
  if (!path.startsWith("/v/")) return null;
  if (request.method !== "GET" && request.method !== "HEAD") return null;

  const id = path.slice("/v/".length);
  if (!ID_RE.test(id)) {
    return new Response("Not found", {
      status: 404,
      headers: { "Content-Type": "text/plain; charset=utf-8" },
    });
  }

  return new Response(LANDING_HTML, {
    status: 200,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      // Static for every id — safe to cache. Short TTL so a copy/UX tweak rolls
      // out quickly.
      "Cache-Control": "public, max-age=300",
      // Lock the page down: no network egress is permitted at all, so even a
      // hypothetical injection can't exfiltrate the fragment key.
      "Content-Security-Policy":
        "default-src 'none'; " +
        "style-src 'unsafe-inline' https://fonts.googleapis.com; " +
        "font-src https://fonts.gstatic.com; " +
        "script-src 'unsafe-inline'; " +
        "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
      "X-Content-Type-Options": "nosniff",
      "X-Frame-Options": "DENY",
      // The fragment is never in a Referer header anyway, but belt-and-braces.
      "Referrer-Policy": "no-referrer",
    },
  });
}

// Single static document. All id/key handling is client-side (see the inline
// script); the server never embeds either. Styling tracks public/sassy-talk.html.
const LANDING_HTML = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SassyTalk Invite</title>
<meta name="robots" content="noindex">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
<style>
  :root {
    --bg-primary: #0a0a0a;
    --bg-secondary: #111111;
    --text-primary: #ffffff;
    --text-secondary: #a0a0a0;
    --text-tertiary: #606060;
    --accent-primary: #06b6d4;
    --accent-green: #22c55e;
    --border-subtle: #222222;
    --border-medium: #333333;
    --gradient-brand: linear-gradient(90deg, #06b6d4, #22c55e);
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
    background: var(--bg-primary);
    color: var(--text-primary);
    line-height: 1.6;
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }
  .card {
    width: 100%;
    max-width: 420px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-subtle);
    border-radius: 20px;
    padding: 2rem 1.75rem;
    text-align: center;
  }
  .logo {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    font-weight: 700;
    font-size: 1.15rem;
    margin-bottom: 1.5rem;
  }
  .logo-icon {
    width: 36px; height: 36px;
    background: var(--gradient-brand);
    border-radius: 9px;
    display: flex; align-items: center; justify-content: center;
    font-size: 18px; color: #000;
  }
  h1 { font-size: 1.45rem; font-weight: 700; margin-bottom: 0.5rem; }
  .sub { color: var(--text-secondary); font-size: 0.95rem; margin-bottom: 1.75rem; }
  .btn {
    display: block;
    width: 100%;
    padding: 0.85rem 1rem;
    border-radius: 12px;
    font-size: 1rem;
    font-weight: 600;
    text-decoration: none;
    border: 1px solid transparent;
    margin-bottom: 0.75rem;
    transition: transform 0.06s ease, opacity 0.2s ease;
  }
  .btn:active { transform: scale(0.99); }
  .btn-primary { background: var(--gradient-brand); color: #000; }
  .btn-secondary {
    background: var(--bg-primary);
    color: var(--text-primary);
    border-color: var(--border-medium);
    cursor: pointer;
    font-family: inherit;
  }
  .btn[aria-disabled="true"] { opacity: 0.4; pointer-events: none; }
  .paste-hint {
    margin-top: 1rem;
    color: var(--text-secondary);
    font-size: 0.82rem;
    line-height: 1.45;
    text-align: left;
  }
  .apk { display: inline-block; margin-top: 0.5rem; font-size: 0.85rem; color: var(--text-tertiary); }
  .apk a { color: var(--accent-primary); text-decoration: none; }
  .note {
    margin-top: 1.5rem;
    padding-top: 1.25rem;
    border-top: 1px solid var(--border-subtle);
    color: var(--text-tertiary);
    font-size: 0.78rem;
  }
  .warn {
    display: none;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.35);
    color: #fca5a5;
    border-radius: 10px;
    padding: 0.7rem 0.85rem;
    font-size: 0.85rem;
    margin-bottom: 1.25rem;
  }
  .lock { color: var(--accent-green); }
</style>
</head>
<body>
  <main class="card">
    <div class="logo"><span class="logo-icon">📡</span><span>SassyTalk</span></div>

    <div id="warn" class="warn">
      This invite link is incomplete — it's missing the part after <code>#</code>.
      Ask the sender to copy and share the whole link again.
    </div>

    <h1>You've been invited</h1>
    <p class="sub">Open this encrypted, one-time session invite in the SassyTalk app.</p>

    <a id="open" class="btn btn-primary" href="#">Open in SassyTalk</a>
    <button id="copy" type="button" class="btn btn-secondary">Copy invite link</button>
    <a class="btn btn-secondary" href="${PLAY_URL}">Get SassyTalk for Android</a>
    <span class="apk">Sideloading? <a href="${APK_URL}">Download the APK</a></span>

    <p id="paste-hint" class="paste-hint" style="display:none">
      Link won't open the app? Copy it above, then in SassyTalk go to
      <strong>Authenticate → Enter Code</strong> and paste the full link.
    </p>

    <p class="note">
      <span class="lock">🔒 End-to-end encrypted.</span>
      The decryption key lives only in this link's <code>#</code> fragment and is
      never sent to our servers — this page can't read your session.
    </p>
  </main>

  <script>
    (function () {
      // The full deep link (including the #key fragment) only exists in the
      // browser's address bar — reconstruct it client-side.
      var open = document.getElementById('open');
      var copyBtn = document.getElementById('copy');
      var pasteHint = document.getElementById('paste-hint');
      var hash = location.hash || '';
      var hasKey = hash.length > 1; // more than a bare '#'
      var fullUrl = location.href;

      function appLinkUrl(href) {
        try {
          var u = new URL(href);
          // u.pathname is already "/v/<id>"; the scheme host is "v", so append
          // only the "/<id>" part — strip the leading "/v" or we'd emit the
          // doubled "sassytalk://v/v/<id>" that the app rejects. The #fragment
          // (the AES key) MUST survive: the custom-scheme intent-filter delivers
          // the whole URL to MainActivity, which reads dataString incl. fragment.
          return 'sassytalk://v' + u.pathname.replace(/^\/v/, '') + u.hash;
        } catch (e) {
          return href;
        }
      }

      if (hasKey) {
        // sassytalk://v/<id>#<key> — opens the installed app on Android AND iOS
        // via the registered custom scheme, no App Links / Universal Links
        // verification needed, and (unlike an intent:// URL) it preserves the
        // #fragment key. If the app isn't installed the tap no-ops and the
        // install buttons below are the fallback.
        var appUrl = appLinkUrl(fullUrl);
        open.setAttribute('href', appUrl);
        open.textContent = 'Open in SassyTalk';
        pasteHint.style.display = 'block';

        copyBtn.addEventListener('click', function () {
          var done = function () {
            copyBtn.textContent = 'Copied!';
            setTimeout(function () { copyBtn.textContent = 'Copy invite link'; }, 2000);
          };
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(fullUrl).then(done).catch(function () {
              window.prompt('Copy this invite link:', fullUrl);
            });
          } else {
            window.prompt('Copy this invite link:', fullUrl);
            done();
          }
        });
      } else {
        open.setAttribute('aria-disabled', 'true');
        copyBtn.setAttribute('aria-disabled', 'true');
        copyBtn.style.opacity = '0.4';
        copyBtn.style.pointerEvents = 'none';
        document.getElementById('warn').style.display = 'block';
      }
    })();
  </script>
</body>
</html>`;
