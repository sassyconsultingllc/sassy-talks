/**
 * SassyConsultingLLC Worker — Main entry point
 *
 * Routes:
 *   /api/ptt/ws?room=...  → Durable Object PTT relay (WebSocket)
 *   /api/analyze           → Connection analysis
 *   /api/contact           → Contact form
 *   /api/checkout          → Stripe checkout
 *   /api/verify            → License verification
 *   /api/webhook           → Stripe webhook
 *   /api/validate          → License validation
 *   /api/vpn-recommendations → VPN list
 *   /api/downloads         → Download catalog
 *   /download/*            → R2 file download
 *   *                      → Static assets (Pages)
 */

import { EmailMessage } from "cloudflare:email";

// Re-export the Durable Object class so the runtime can find it
export { PttRoom } from "./ptt-relay.js";

// ── Product catalog ──
const PRODUCTS = {
  "sassy-talk": {
    name: "Sassy-Talk",
    amount: 200,
    description: "Encrypted walkie-talkie app for Android and Windows",
  },
  "winforensics": {
    name: "WinForensics",
    amount: 200,
    description: "Digital forensics toolkit for Windows",
  },
  "website-creator": {
    name: "Website Creator",
    amount: 200,
    description: "AI-powered WordPress builder with security hardening",
  },
};

// ── Datacenter/VPN detection ──
const DATACENTER_ASNS = new Set([
  13335, 14618, 15169, 8075, 16509, 14061, 20473, 46606, 63949, 54825,
  398101, 13213, 32934, 19551, 36351, 30633, 21859,
]);
const VPN_KEYWORDS = [
  "vpn", "proxy", "hosting", "datacenter", "data center", "cloud", "server",
  "vps", "dedicated", "colocation", "aws", "amazon", "google", "microsoft",
  "azure", "digitalocean", "linode", "vultr", "ovh", "hetzner", "cloudflare",
  "akamai",
];

// ═══════════════════════════════════════════════════════════════════════════
// Main fetch handler
// ═══════════════════════════════════════════════════════════════════════════

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const path = url.pathname;
    const method = request.method;

    const corsHeaders = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type, stripe-signature",
    };

    // Security headers applied to all responses via wrapper
    const securityHeaders = {
      "X-Content-Type-Options": "nosniff",
      "X-Frame-Options": "DENY",
      "Referrer-Policy": "strict-origin-when-cross-origin",
      "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
      "Content-Security-Policy": "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self' https://api.stripe.com",
      "Strict-Transport-Security": "max-age=31536000; includeSubDomains; preload",
    };

    if (method === "OPTIONS") {
      return new Response(null, { headers: { ...corsHeaders, ...securityHeaders } });
    }

    try {
      let response;

      // ── PTT WebSocket relay ──
      if (path === "/api/ptt/ws") {
        return handlePttWebSocket(request, env, url);
      }

      if (path === "/api/analyze" && method === "POST") {
        response = await handleAnalyze(request, env, corsHeaders);
      } else if (path === "/api/contact" && method === "POST") {
        response = await handleContact(request, env, corsHeaders);
      } else if (path === "/api/checkout" && method === "POST") {
        response = await handleCheckout(request, env, corsHeaders);
      } else if (path === "/api/verify" && method === "POST") {
        response = await handleVerify(request, env, corsHeaders);
      } else if (path === "/api/webhook" && method === "POST") {
        response = await handleWebhook(request, env, corsHeaders);
      } else if (path === "/api/validate" && method === "POST") {
        response = await handleValidateLicense(request, env, corsHeaders);
      } else if (path === "/api/vpn-recommendations") {
        response = await handleVPNRecommendations(corsHeaders);
      } else if (path === "/api/downloads") {
        response = await handleDownloadsList(env, corsHeaders);
      } else if (path.startsWith("/download/")) {
        response = await handleDownload(path, env, corsHeaders);
      } else if (path === "/privacy-policy.html" || path.startsWith("/legal/")) {
        response = await handlePrivacyPage(path, env, corsHeaders);
      } else {
        response = await env.ASSETS.fetch(request);
      }

      return addSecurityHeaders(response, securityHeaders);
    } catch (error) {
      return new Response(JSON.stringify({ error: "Internal server error" }), {
        status: 500,
        headers: { ...corsHeaders, ...securityHeaders, "Content-Type": "application/json" },
      });
    }
  },
};

/**
 * Clone a response and add security headers.
 * Skips WebSocket upgrade responses (status 101).
 */
function addSecurityHeaders(response, securityHeaders) {
  if (response.status === 101) return response;
  const newResponse = new Response(response.body, response);
  for (const [key, value] of Object.entries(securityHeaders)) {
    newResponse.headers.set(key, value);
  }
  return newResponse;
}

// ═══════════════════════════════════════════════════════════════════════════
// PTT WebSocket relay — routes to Durable Object by room ID
// ═══════════════════════════════════════════════════════════════════════════

function handlePttWebSocket(request, env, url) {
  const upgradeHeader = request.headers.get("Upgrade");
  if (!upgradeHeader || upgradeHeader !== "websocket") {
    return new Response("Expected WebSocket upgrade", { status: 426 });
  }

  const room = url.searchParams.get("room");
  if (!room || room.length < 8 || room.length > 128) {
    return new Response(JSON.stringify({ error: "Invalid room parameter" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Sanitize: only allow alphanumeric, hyphens, underscores
  if (!/^[a-zA-Z0-9_-]+$/.test(room)) {
    return new Response(JSON.stringify({ error: "Invalid room format" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Each room gets its own Durable Object instance (keyed by room name).
  // All devices in the same QR session join the same room.
  const id = env.PTT_RELAY.idFromName(room);
  const stub = env.PTT_RELAY.get(id);

  // Forward the request to the Durable Object — it handles the WS upgrade
  return stub.fetch(request);
}

// ═══════════════════════════════════════════════════════════════════════════
// Existing API handlers (preserved from current deployed Worker)
// ═══════════════════════════════════════════════════════════════════════════

async function handleAnalyze(request, env, corsHeaders) {
  const body = await safeParseJSON(request, 2048);
  if (!body) {
    return new Response(JSON.stringify({ error: "Invalid request" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  const zipCode = (body.zip_code || "").toString().substring(0, 10);
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  const country = request.headers.get("CF-IPCountry") || "XX";
  const city = request.headers.get("CF-IPCity") || "Unknown";
  const region = request.headers.get("CF-Region") || "";
  const lat = request.headers.get("CF-IPLatitude") || "0";
  const lon = request.headers.get("CF-IPLongitude") || "0";
  const asn = request.headers.get("CF-IPAsn") || "";
  const asnOrg = request.headers.get("CF-IPAsnOrg") || "Unknown ISP";

  const vpnDetection = detectVPN(asn, asnOrg, request.headers);
  const maskedIP = maskIP(ip);

  const pingStart = Date.now();
  try {
    await fetch("https://bitdefender.com", {
      method: "HEAD",
      signal: AbortSignal.timeout(3000),
    });
  } catch (e) {}
  const pingMs = Date.now() - pingStart;

  if (env.DB) {
    const hashedIP = await hashIP(ip, env.LICENSE_SALT || "default-salt");
    try {
      await env.DB.prepare(
        `INSERT INTO connection_logs (ip_hash, zip_code, country, region, asn, is_vpn, created_at)
         VALUES (?, ?, ?, ?, ?, ?, datetime('now'))`
      )
        .bind(hashedIP, zipCode, country, region, asn, vpnDetection.isVPN ? 1 : 0)
        .run();
    } catch (e) {}
  }

  let connectionStatus = "safe";
  let statusMessage = "Connection appears secure";
  if (vpnDetection.isVPN) {
    connectionStatus = "protected";
    statusMessage = "VPN or proxy detected - your traffic is being routed";
  } else if (country !== "US") {
    connectionStatus = "warning";
    statusMessage = "International connection detected";
  }

  const response = {
    connection_status: connectionStatus,
    status_message: statusMessage,
    ip: maskedIP,
    ip_full_masked: maskedIP,
    location: {
      city,
      region,
      country,
      postal: zipCode,
      latitude: parseFloat(lat),
      longitude: parseFloat(lon),
      timezone: getTimezone(country, region),
    },
    isp: { name: asnOrg, asn: asn ? `AS${asn}` : "Unknown" },
    vpn: vpnDetection,
    ping_ms: pingMs,
    input_zip: zipCode,
  };

  return new Response(JSON.stringify(response), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

async function handleContact(request, env, corsHeaders) {
  const contentType = request.headers.get("content-type") || "";
  let name, email, message;

  if (contentType.includes("application/x-www-form-urlencoded")) {
    const text = await request.text();
    if (text.length > 4096) {
      return new Response(null, { status: 302, headers: { Location: "/#contact?error=size" } });
    }
    const formData = new URLSearchParams(text);
    name = formData.get("name");
    email = formData.get("email");
    message = formData.get("message");
  } else {
    const body = await safeParseJSON(request, 4096);
    if (!body) {
      return new Response(null, { status: 302, headers: { Location: "/#contact?error=invalid" } });
    }
    name = body.name;
    email = body.email;
    message = body.message;
  }

  const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  if (!name || name.length < 2 || name.length > 100) {
    return new Response(null, { status: 302, headers: { Location: "/#contact?error=name" } });
  }
  if (!email || !emailRegex.test(email)) {
    return new Response(null, { status: 302, headers: { Location: "/#contact?error=email" } });
  }
  if (!message || message.length < 10 || message.length > 1000) {
    return new Response(null, { status: 302, headers: { Location: "/#contact?error=message" } });
  }

  if (env.DB) {
    try {
      await env.DB.prepare(
        `INSERT INTO contact_submissions (name, email, message, created_at)
         VALUES (?, ?, ?, datetime('now'))`
      )
        .bind(name, email, message)
        .run();
    } catch (e) {
      console.error("DB error:", e);
    }
  }

  if (env.CONTACT_EMAIL) {
    try {
      const emailBody = [
        `From: Info@sassyconsultingllc.com`,
        `To: Info@sassyconsultingllc.com`,
        `Reply-To: ${email}`,
        `Subject: Contact Form: ${name}`,
        `Content-Type: text/plain; charset=utf-8`,
        ``,
        `New contact form submission:`,
        ``,
        `Name: ${name}`,
        `Email: ${email}`,
        ``,
        `Message:`,
        `${message}`,
        ``,
        `---`,
        `Sent from sassyconsultingllc.com contact form`,
      ].join("\r\n");

      const msg = new EmailMessage(
        "Info@sassyconsultingllc.com",
        "Info@sassyconsultingllc.com",
        emailBody
      );
      await env.CONTACT_EMAIL.send(msg);
    } catch (e) {
      console.error("Email error:", e);
    }
  }

  return new Response(null, { status: 302, headers: { Location: "/contact-success.html" } });
}

async function handleCheckout(request, env, corsHeaders) {
  const body = await safeParseJSON(request, 4096);
  if (!body) {
    return new Response(JSON.stringify({ error: "Invalid request" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  const { product, email, success_url, cancel_url } = body;

  if (!product || !PRODUCTS[product]) {
    return new Response(JSON.stringify({ error: "Invalid product" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  if (!email || typeof email !== "string" || !emailRegex.test(email) || email.length > 254) {
    return new Response(JSON.stringify({ error: "Valid email required" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  // Validate URLs if provided — only allow same-origin
  if (success_url && typeof success_url === "string") {
    try {
      const u = new URL(success_url);
      if (u.origin !== new URL(request.url).origin) {
        return new Response(JSON.stringify({ error: "Invalid success_url" }), {
          status: 400,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        });
      }
    } catch {
      return new Response(JSON.stringify({ error: "Invalid success_url" }), {
        status: 400,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      });
    }
  }
  if (cancel_url && typeof cancel_url === "string") {
    try {
      const u = new URL(cancel_url);
      if (u.origin !== new URL(request.url).origin) {
        return new Response(JSON.stringify({ error: "Invalid cancel_url" }), {
          status: 400,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        });
      }
    } catch {
      return new Response(JSON.stringify({ error: "Invalid cancel_url" }), {
        status: 400,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      });
    }
  }

  const productInfo = PRODUCTS[product];
  const priceId = env[`STRIPE_PRICE_${product.toUpperCase().replace("-", "_")}`];

  const stripeResponse = await fetch("https://api.stripe.com/v1/checkout/sessions", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.STRIPE_SECRET_KEY}`,
      "Content-Type": "application/x-www-form-urlencoded",
    },
    body: new URLSearchParams({
      "payment_method_types[]": "card",
      "line_items[0][price]": priceId,
      "line_items[0][quantity]": "1",
      mode: "payment",
      success_url: success_url || `${env.SITE_URL}/success?session_id={CHECKOUT_SESSION_ID}`,
      cancel_url: cancel_url || `${env.SITE_URL}/${product}.html`,
      customer_email: email,
      "metadata[product]": product,
      "metadata[product_name]": productInfo.name,
    }),
  });

  const session = await stripeResponse.json();
  if (session.error) {
    return new Response(JSON.stringify({ error: session.error.message }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  return new Response(
    JSON.stringify({ checkout_url: session.url, session_id: session.id }),
    { headers: { ...corsHeaders, "Content-Type": "application/json" } }
  );
}

async function handleVerify(request, env, corsHeaders) {
  const body = await safeParseJSON(request);
  if (!body) {
    return new Response(JSON.stringify({ error: "Invalid request" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  const { session_id } = body;

  if (!session_id || !isValidStripeSessionId(session_id)) {
    return new Response(JSON.stringify({ error: "Invalid session ID" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  const stripeResponse = await fetch(
    `https://api.stripe.com/v1/checkout/sessions/${encodeURIComponent(session_id)}`,
    { headers: { Authorization: `Bearer ${env.STRIPE_SECRET_KEY}` } }
  );
  const session = await stripeResponse.json();

  if (session.error) {
    return new Response(JSON.stringify({ error: session.error.message }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  if (session.payment_status !== "paid") {
    return new Response(JSON.stringify({ error: "Payment not completed" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  const product = session.metadata.product;
  const email = session.customer_email;
  const licenseKey = await generateLicenseKey(email, product, session_id, env.LICENSE_SALT);

  if (env.DB) {
    try {
      await env.DB.prepare(
        `INSERT INTO licenses (license_key, email, product, stripe_session_id, created_at)
         VALUES (?, ?, ?, ?, datetime('now'))`
      )
        .bind(licenseKey, email, product, session_id)
        .run();
    } catch (e) {}
  }

  return new Response(
    JSON.stringify({
      success: true,
      license_key: licenseKey,
      product,
      product_name: PRODUCTS[product]?.name || product,
      email,
    }),
    { headers: { ...corsHeaders, "Content-Type": "application/json" } }
  );
}

async function handleWebhook(request, env, corsHeaders) {
  // Stripe webhooks can be up to ~64KB; limit to 128KB for safety
  const contentLength = parseInt(request.headers.get("Content-Length") || "0");
  if (contentLength > 131072) {
    return new Response(JSON.stringify({ error: "Payload too large" }), {
      status: 413,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  const payload = await request.text();
  if (payload.length > 131072) {
    return new Response(JSON.stringify({ error: "Payload too large" }), {
      status: 413,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  let event;
  try {
    event = JSON.parse(payload);
  } catch {
    return new Response(JSON.stringify({ error: "Invalid JSON" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  if (!event || typeof event.type !== "string") {
    return new Response(JSON.stringify({ error: "Invalid event" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  if (event.type === "checkout.session.completed") {
    const session = event.data.object;
    const product = session.metadata.product;
    const email = session.customer_email;
    const licenseKey = await generateLicenseKey(email, product, session.id, env.LICENSE_SALT);

    if (env.DB) {
      try {
        await env.DB.prepare(
          `INSERT OR IGNORE INTO licenses (license_key, email, product, stripe_session_id, created_at)
           VALUES (?, ?, ?, ?, datetime('now'))`
        )
          .bind(licenseKey, email, product, session.id)
          .run();
      } catch (e) {}
    }
  }

  return new Response(JSON.stringify({ received: true }), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

async function handleValidateLicense(request, env, corsHeaders) {
  const body = await safeParseJSON(request);
  if (!body) {
    return new Response(JSON.stringify({ valid: false, error: "Invalid request" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  const { license_key } = body;

  // Validate format: SASSY-XXXX-XXXX-XXXX-XXXX (alphanumeric + hyphens, max 30 chars)
  if (!license_key || typeof license_key !== "string" || license_key.length > 30) {
    return new Response(JSON.stringify({ valid: false, error: "License key required" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  if (!/^SASSY-[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}$/.test(license_key)) {
    return new Response(JSON.stringify({ valid: false, error: "Invalid license format" }), {
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  if (env.DB) {
    const result = await env.DB.prepare(
      `SELECT product, email, created_at FROM licenses WHERE license_key = ?`
    )
      .bind(license_key)
      .first();

    if (result) {
      return new Response(
        JSON.stringify({ valid: true, product: result.product, created_at: result.created_at }),
        { headers: { ...corsHeaders, "Content-Type": "application/json" } }
      );
    }
  }

  return new Response(JSON.stringify({ valid: false, error: "License not found" }), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

async function handleVPNRecommendations(corsHeaders) {
  const recommendations = [
    { name: "ProtonVPN", description: "Swiss-based, strict no-log policy, open source apps", website: "https://protonvpn.com", free_tier: true },
    { name: "Windscribe", description: "10GB free per month, browser extension included", website: "https://windscribe.com", free_tier: true },
    { name: "Cloudflare WARP", description: "Fast and lightweight, built into 1.1.1.1 app", website: "https://1.1.1.1", free_tier: true },
    { name: "TunnelBear", description: "Simple interface, 2GB free per month", website: "https://tunnelbear.com", free_tier: true },
  ];
  return new Response(JSON.stringify(recommendations), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

async function handleDownloadsList(env, corsHeaders) {
  const downloads = [
    {
      product: "sassy-talk",
      name: "Sassy-Talk",
      platforms: [
        { platform: "android", filename: "sassytalkie.apk", size: "15MB" },
        { platform: "windows", filename: "sassy-talk-setup.msi", size: "25MB" },
      ],
    },
    {
      product: "winforensics",
      name: "WinForensics",
      platforms: [{ platform: "windows", filename: "winforensics-setup.msi", size: "18MB" }],
    },
  ];
  return new Response(JSON.stringify(downloads), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

async function handleDownload(path, env, corsHeaders) {
  const parts = path.replace("/download/", "").split("/");
  if (parts.length < 3) {
    return new Response("Not found", { status: 404 });
  }
  const [product, platform, filename] = parts;

  // Prevent path traversal — no dots-dots, no slashes in segments
  const safeSegment = /^[a-zA-Z0-9._-]+$/;
  if (!safeSegment.test(product) || !safeSegment.test(platform) || !safeSegment.test(filename)) {
    return new Response("Invalid path", { status: 400 });
  }
  if (product.includes("..") || platform.includes("..") || filename.includes("..")) {
    return new Response("Invalid path", { status: 400 });
  }

  const r2Key = `${product}/${platform}/${filename}`;

  const object = await env.DOWNLOADS.get(r2Key);
  if (!object) {
    return new Response("File not found", { status: 404 });
  }

  if (env.DB) {
    try {
      await env.DB.prepare(
        `UPDATE downloads SET download_count = download_count + 1 WHERE r2_key = ?`
      )
        .bind(r2Key)
        .run();
    } catch (e) {}
  }

  const headers = new Headers(corsHeaders);
  headers.set("Content-Type", object.httpMetadata?.contentType || "application/octet-stream");
  headers.set("Content-Disposition", `attachment; filename="${filename}"`);
  return new Response(object.body, { headers });
}

async function handlePrivacyPage(path, env, corsHeaders) {
  // Map URL path to R2 key — prevent traversal
  if (path.includes("..") || path.includes("//")) {
    return new Response("Not found", { status: 404 });
  }

  let r2Key;
  if (path === "/privacy-policy.html") {
    r2Key = "privacy-policy.html";
  } else {
    // /legal/foo.html → legal/foo.html (only allow simple paths)
    r2Key = path.replace(/^\//, "");
    if (!/^legal\/[a-zA-Z0-9._-]+$/.test(r2Key)) {
      return new Response("Not found", { status: 404 });
    }
  }

  const object = await env.PRIVACY.get(r2Key);
  if (!object) {
    return new Response("Not found", { status: 404 });
  }

  const headers = new Headers(corsHeaders);
  headers.set("Content-Type", object.httpMetadata?.contentType || "text/html; charset=utf-8");
  headers.set("Cache-Control", "public, max-age=3600");
  return new Response(object.body, { headers });
}

// ═══════════════════════════════════════════════════════════════════════════
// Security & validation helpers
// ═══════════════════════════════════════════════════════════════════════════

/** Parse JSON body with size limit (default 10KB). Returns null if too large or invalid. */
async function safeParseJSON(request, maxBytes = 10240) {
  const contentLength = parseInt(request.headers.get("Content-Length") || "0");
  if (contentLength > maxBytes) return null;
  try {
    const text = await request.text();
    if (text.length > maxBytes) return null;
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** Validate a Stripe session ID format (cs_test_... or cs_live_...) */
function isValidStripeSessionId(id) {
  return typeof id === "string" && /^cs_(test|live)_[a-zA-Z0-9]{10,200}$/.test(id);
}

// ═══════════════════════════════════════════════════════════════════════════
// Utility functions
// ═══════════════════════════════════════════════════════════════════════════

function detectVPN(asn, asnOrg, headers) {
  const asnNum = parseInt(asn);
  const orgLower = asnOrg.toLowerCase();

  if (DATACENTER_ASNS.has(asnNum)) {
    return { isVPN: true, confidence: "high", reason: "Known datacenter ASN" };
  }
  for (const keyword of VPN_KEYWORDS) {
    if (orgLower.includes(keyword)) {
      return { isVPN: true, confidence: "medium", reason: `ISP name contains "${keyword}"` };
    }
  }
  const proxyHeaders = ["X-Forwarded-For", "Via", "X-Proxy-ID", "Forwarded"];
  for (const header of proxyHeaders) {
    if (headers.get(header)) {
      return { isVPN: true, confidence: "medium", reason: "Proxy headers detected" };
    }
  }
  return { isVPN: false, confidence: "high", reason: "Direct connection" };
}

function maskIP(ip) {
  if (!ip || ip === "unknown") return "xxx.xxx";
  const parts = ip.split(".");
  if (parts.length === 4) return `${parts[0]}.${parts[1]}.xxx.xxx`;
  return "xxx.xxx";
}

async function hashIP(ip, salt) {
  const data = new TextEncoder().encode(ip + salt);
  const hashBuffer = await crypto.subtle.digest("SHA-256", data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function generateLicenseKey(email, product, orderId, salt) {
  const raw = `${email}:${product}:${orderId}:${salt || "default"}`;
  const data = new TextEncoder().encode(raw);
  const hashBuffer = await crypto.subtle.digest("SHA-256", data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  const hex = hashArray.map((b) => b.toString(16).padStart(2, "0")).join("").toUpperCase();
  const productCode = product.toUpperCase().replace("-", "").substring(0, 4);
  return `SASSY-${productCode}-${hex.substring(0, 4)}-${hex.substring(4, 8)}-${hex.substring(8, 12)}`;
}

function getTimezone(country, region) {
  const timezones = {
    US: {
      CA: "America/Los_Angeles",
      NY: "America/New_York",
      TX: "America/Chicago",
      WI: "America/Chicago",
      default: "America/New_York",
    },
    default: "UTC",
  };
  const countryTz = timezones[country] || timezones["default"];
  if (typeof countryTz === "object") return countryTz[region] || countryTz["default"];
  return countryTz;
}
