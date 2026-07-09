import type { EngagementId } from "./control-plane-domain";
import type { RouteJson } from "./control-plane-transport";

/** Managed-auth sign-in for an embedded end-user: returns the audience session
 *  token carried on subsequent embed calls. The real magic-link/social flow is
 *  hosted; this is the dev path. */
export async function embedSignin(
    json: RouteJson,
    email: string,
): Promise<{ token: string; audience: string }> {
    return (await json("POST", "/embed/signin", { email })) as {
        token: string;
        audience: string;
    };
}

/** Persist an audience-keyed durable chat (requires the audience bearer). */
export async function embedCreateChat(json: RouteJson, title: string): Promise<{ chat: string }> {
    return (await json("POST", "/embed/chats", { title })) as { chat: string };
}

/** The signed-in end-user's own durable chats — fail-closed, scoped to the bearer. */
export async function embedMyChats(json: RouteJson): Promise<{ chat: string; title: string }[]> {
    const o = (await json("GET", "/embed/my-chats")) as {
        chats: { chat: string; title: string }[];
    };
    return o.chats;
}

/** The deployment's public embed config the panel reads to honor white-label (EMBED-7):
 *  when `white_label` is set, the panel suppresses the "Powered by GaugeDesk" mark. */
export async function embedGetConfig(json: RouteJson): Promise<{ white_label: boolean }> {
    const o = (await json("GET", "/embed/config")) as { config?: { white_label?: boolean } };
    return { white_label: Boolean(o.config?.white_label) };
}

/** Drive a public embed visitor turn (`POST /embed/sessions/:id/turn`). */
export async function runEmbedTurn(
    json: RouteJson,
    id: EngagementId,
    prompt: string,
    images: { data: string; mimeType: string }[] = [],
): Promise<unknown> {
    const body = images.length ? { prompt, images } : { prompt };
    return json("POST", `/embed/sessions/${id}/turn`, body);
}
