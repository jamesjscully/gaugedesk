/**
 * The embed example/demo page bootstrap (EMBED-2). It registers the custom
 * elements, then composes a `<gw-session>` with all three panels, pointed at a
 * scoped control plane. `?cp=` overrides the backend base; `?engagement=` binds an
 * existing chat — and absent one (the demo/e2e case) it spins up a fresh work chat
 * via the same quick-start the workbench uses, so the page is self-contained.
 */
import { registerEmbedElements } from "./elements";
import { EmbedControlPlane, controlPlaneBase } from "./embed-control-plane";

registerEmbedElements();

async function main() {
    const params = new URLSearchParams(location.search);
    const base = params.get("cp") ?? controlPlaneBase();
    // `?auth=1` demos the authenticated mode (EMBED-4/5): sign in a managed-auth
    // end-user, persist a durable chat, and show my-chats. Default is anonymous.
    const authed = params.get("auth") === "1";

    let token: string | undefined;
    if (authed) {
        const api = new EmbedControlPlane(base);
        const signin = await api.embedSignin("demo@reader.example");
        token = signin.token;
        api.setBearer(token);
        await api.embedCreateChat("my saved chapter");
    }

    let engagement = params.get("engagement");
    if (!engagement) {
        const api = new EmbedControlPlane(base);
        const eng = await api.createEngagement();
        engagement = String(eng.id);
    }

    const session = document.createElement("gw-session");
    session.setAttribute("cp", base);
    session.setAttribute("engagement", engagement);
    if (token) session.setAttribute("token", token);
    // Authenticated: chat + the my-chats listing. Anonymous: the full panel set.
    session.innerHTML = authed
        ? "<gw-chat></gw-chat><gw-chats></gw-chats>"
        : "<gw-chat></gw-chat><gw-viewer></gw-viewer><gw-files></gw-files>";

    const mount = document.getElementById("mount");
    if (mount) mount.appendChild(session);
}

void main();
