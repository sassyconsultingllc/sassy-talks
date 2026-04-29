// Minimal stub for cloudflare:workers so unit tests can import ptt-relay.js
// without a full Cloudflare Workers runtime.
export class DurableObject {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }
}
