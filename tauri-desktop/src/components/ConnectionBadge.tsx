// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-JSWQWRGEIIOS
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Quality {
  health: string;
  rtt?: number;
  transport: string;
}

export function ConnectionBadge({ peerId }: { peerId: string }) {
  const [q, setQ] = useState<Quality | null>(null);

  useEffect(() => {
    let mounted = true;
    const id = setInterval(async () => {
      try {
        const all = await invoke<[string, string, number | null, string][]>("get_connection_quality");
        if (!mounted) return;
        const mine = all.find(([id]) => id === peerId);
        if (mine) {
          setQ({ health: mine[1], rtt: mine[2] ?? undefined, transport: mine[3] });
        }
      } catch {}
    }, 1000);
    return () => {
      mounted = false;
      clearInterval(id);
    };
  }, [peerId]);

  if (!q) return null;

  const color = q.health === "healthy" ? "#4CAF50" : q.health === "degraded" ? "#FFC107" : "#F44336";
  const icon = q.transport === "BLE" ? "🔵" : q.transport === "UDP" ? "📡" : "🌐";

  return (
    <span style={{ color, fontSize: 12, marginLeft: 8 }}>
      {icon} {q.transport} {q.rtt !== undefined && q.rtt > 0 ? `${q.rtt}ms` : ""}
      {q.health === "stale" && " ⚠️"}
    </span>
  );
}
