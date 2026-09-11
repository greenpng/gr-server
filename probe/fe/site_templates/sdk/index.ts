/**
 * greenv5 first-party Next.js SDK surface.
 *
 * Components (browser boot cfg):
 *   MaxProbeHead / MaxProbeCollector — set apiBase=/g5, first_party=1
 *
 * Edge/API (same-origin stream proxy — must not reparse body):
 *   createRelayHandlers / relayRequest
 *
 * Install into a Next 13+ app:
 *   1. Copy MaxProbeHead + MaxProbeCollector into components/
 *   2. Copy sdk/g5 and sdk/g5-gw under app/  (or re-export route handlers)
 *   3. Env:
 *        NEXT_PUBLIC_MAX_PROBE_BASE=/g5
 *        NEXT_PUBLIC_MAX_PROBE_GW_BASE=/g5-gw
 *        NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY=1
 *        GR_INGEST_UPSTREAM=http://127.0.0.1:28765
 *        GR_GW_UPSTREAM=http://127.0.0.1:28766
 *   4. Prefer nginx reverse proxy for /g5 when available (see scripts/nginx_g5_first_party_snippet.conf).
 *      Next route remains the portable path; latency: nginx < Next stream < CF→pv.
 */

export { relayRequest, createRelayHandlers } from './relay';
export type { RelayRole } from './relay';
