/**
 * A tiny wrapper over `qrcode-generator` (a pure-JS encoder, no network) that
 * renders a string as an inline SVG. Used to show a pairing / hand-off invite as a
 * scannable code (FED-7): the desktop renders the QR; native camera **scan** is the
 * deferred `D-MOBILE` half — any phone QR reader yields the `gaugewright://…` link today.
 */

import qrcode from "qrcode-generator";

/**
 * An `<svg>` string encoding `text` as a QR code, sized to `cellSize` px per module.
 * Error-correction level **M** (the usual balance); auto type number (`0`) grows the
 * grid to fit the payload. Render with `innerHTML` (the markup is library-generated,
 * not user HTML).
 */
export function qrSvg(text: string, cellSize = 4): string {
    const qr = qrcode(0, "M");
    qr.addData(text);
    qr.make();
    return qr.createSvgTag({ cellSize, margin: 2, scalable: true });
}
