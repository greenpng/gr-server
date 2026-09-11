/**
 * First-party gateway relay: /g5-gw/* → GR_GW_UPSTREAM (default :28766)
 * B8 early / s0 / gateway paths — same stream rules as ingest relay.
 */
import { createRelayHandlers } from '../../relay';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';
export const maxDuration = 30;

export const { GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD } =
  createRelayHandlers('gateway');
