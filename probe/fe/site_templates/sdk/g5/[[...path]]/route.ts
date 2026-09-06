/**
 * First-party ingest/API relay: /g5/* → GV5_INGEST_UPSTREAM (default :28765)
 *
 * Copy into Next site as: app/g5/[[...path]]/route.ts
 * Or re-export:
 *   export { GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD } from '@/…/sdk/g5/[[...path]]/route'
 *
 * Prefer nginx `location /g5/` in production (lower latency). Keep this route as
 * portable fallback so CF orange-cloud never forces browser→pv CORS/challenge path.
 */
import { createRelayHandlers } from '../../relay';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';
/** Allow long probe uploads without edge function hard-kill (platform-dependent). */
export const maxDuration = 60;

export const { GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD } =
  createRelayHandlers('ingest');
