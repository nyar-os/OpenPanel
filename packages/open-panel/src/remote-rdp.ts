/**
 * @deprecated Import from `./remote-ironrdp.js`. Kept as a thin re-export so stale
 * FreeRDP wrappers cannot return. External RDP CLIs are forbidden.
 */
export {
    rdpHandshake,
    rdpProbe,
    resolveRdpBin,
    type RdpHandshakeInput,
    type RdpHandshakeReport,
} from './remote-ironrdp.js';
