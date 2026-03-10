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
export { PttRelay } from "./ptt-relay.js";

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

    if (method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders });
    }

    try {
      // ── PTT WebSocket relay ──
      if (path === "/api/ptt/ws") {
        return handlePttWebSocket(request, env, url);
      }

      if (path === "/api/analyze" && method === "POST") {
        return await handleAnalyze(request, env, corsHeaders);
      }
      if (path === "/api/contact" && method === "POST") {
        return await handleContact(request, env, corsHeaders);
      }
      if (path === "/api/checkout" && method === "POST") {
        return await handleCheckout(request, env, corsHeaders);
      }
      if (path === "/api/verify" && method === "POST") {
        return await handleVerify(request, env, corsHeaders);
      }
      if (path === "/api/webhook" && method === "POST") {
        return await handleWebhook(request, env, corsHeaders);
      }
      if (path === "/api/validate" && method === "POST") {
        return await handleValidateLicense(request, env, corsHeaders);
      }
      if (path === "/api/vpn-recommendations") {
        return await handleVPNRecommendations(corsHeaders);
      }
      if (path === "/api/downloads") {
        return await handleDownloadsList(env, corsHeaders);
      }
      if (path.startsWith("/download/")) {
        return await handleDownload(path, env, corsHeaders);
      }

      return env.ASSETS.fetch(request);
    } catch (error) {
      return new Response(JSON.stringify({ error: error.message }), {
        status: 500,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      });
    }
  },
};

// ═══════════════════════════════════════════════════════════════════════════
// PTT WebSocket relay — routes to Durable Object by room ID
// ═══════════════════════════════════════════════════════════════════════════

function handlePttWebSocket(request, env, url) {
  const upgradeHeader = request.headers.get("Upgrade");
  if (!upgradeHeader || upgradeHeader !== "websocket") {
    return new Response("Expected WebSocket upgrade", { status: 426 });
  }

  const room = url.searchParams.get("room");
  if (!room) {
    return new Response(JSON.stringify({ error: "Missing room parameter" }), {
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
  const body = await request.json();
  const zipCode = body.zip_code || "";
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
    const formData = await request.formData();
    name = formData.get("name");
    email = formData.get("email");
    message = formData.get("message");
  } else {
    const body = await request.json();
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
  const body = await request.json();
  const { product, email, success_url, cancel_url } = body;

  if (!product || !PRODUCTS[product]) {
    return new Response(JSON.stringify({ error: "Invalid product" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  if (!email) {
    return new Response(JSON.stringify({ error: "Email required" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
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
  const body = await request.json();
  const { session_id } = body;

  if (!session_id) {
    return new Response(JSON.stringify({ error: "Session ID required" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }

  const stripeResponse = await fetch(
    `https://api.stripe.com/v1/checkout/sessions/${session_id}`,
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
  const payload = await request.text();
  const event = JSON.parse(payload);

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
  const body = await request.json();
  const { license_key } = body;

  if (!license_key) {
    return new Response(JSON.stringify({ valid: false, error: "License key required" }), {
      status: 400,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
  }
  if (!license_key.startsWith("SASSY-")) {
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
