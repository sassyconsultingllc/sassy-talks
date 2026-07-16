// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-QVHMWW5TEOQS
// Minimal stub for cloudflare:workers so unit tests can import ptt-relay.js
// without a full Cloudflare Workers runtime.
export class DurableObject {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }
}
