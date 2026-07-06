// Vite's `?inline` query returns a stylesheet's text as a string (rather than
// injecting it). The embed elements adopt it into each panel's shadow root so the
// workbench styles apply inside the isolated tree (EMBED-2).
declare module "*.css?inline" {
    const css: string;
    export default css;
}
