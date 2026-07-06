/**
 * The embed entry (EMBED-2): importing this registers the `<gw-session>` /
 * `<gw-chat>` / `<gw-viewer>` / `<gw-files>` custom elements. This module is the
 * source for the eventual `embed.js` bundle a consultant drops onto their page
 * (the bundle/CDN itself is needs-infra, `D-HOST`).
 */
import { registerEmbedElements } from "./elements";

registerEmbedElements();

export { registerEmbedElements } from "./elements";
export { GwSessionElement, GwChatElement, GwViewerElement, GwFilesElement, GwChatsElement } from "./elements";
export { EmbedControlPlane, controlPlaneBase } from "./embed-control-plane";
export type { EmbedSessionApi } from "./embed-control-plane";
export { createRemoteSession } from "./remote-session";
export type { RemoteSessionOptions } from "./remote-session";
export { SessionProvider, useSession } from "@gaugewright/workbench-ui/session-context";
export type { Session } from "@gaugewright/workbench-ui/session-context";
