/**
 * SassyTalk PTT Relay — Dedicated Worker
 *
 * Pure WebSocket relay for encrypted audio. No website, no APIs, no assets.
 * Lives at relay.sassy-consults.com
 */

export { PttRoom } from "./ptt-relay.js";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    // CORS
    if (request.method === "OPTIONS") {
      return new Response(null, {
        headers: {
          "Access-Control-Allow-Origin": "*",
          "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
          "Access-Control-Allow-Headers": "Content-Type",
        },
      });
    }

    // Health check
    if (path === "/" || path === "/health") {
      return new Response(JSON.stringify({
        service: "sassytalk-relay",
        status: "ok",
        max_peers_per_room: 16,
      }), {
        headers: { "Content-Type": "application/json" },
      });
    }

    // WebSocket relay
    if (path === "/ws" || path === "/api/ptt/ws") {
      const roomId = url.searchParams.get("room");
      if (!roomId || roomId.length < 8 || roomId.length > 64) {
        return new Response("Missing or invalid room ID", { status: 400 });
      }

      const doId = env.PTT_RELAY.idFromName(roomId);
      const room = env.PTT_RELAY.get(doId);
      return room.fetch(request);
    }

    return new Response("Not found", { status: 404 });
  },
};
