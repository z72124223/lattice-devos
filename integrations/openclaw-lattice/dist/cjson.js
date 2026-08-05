export class CanonicalJsonError extends Error {
    constructor(message) {
        super(message);
        this.name = "CanonicalJsonError";
    }
}
export function canonicalJson(value) {
    if (value === null) {
        return "null";
    }
    if (typeof value === "boolean") {
        return value ? "true" : "false";
    }
    if (typeof value === "string") {
        return encodeString(normalize(value));
    }
    if (Array.isArray(value)) {
        const entries = value;
        return `[${entries.map((entry) => canonicalJson(entry)).join(",")}]`;
    }
    if (typeof value === "object") {
        const normalized = Object.entries(value).map(([key, entry]) => [
            normalize(key),
            entry,
        ]);
        normalized.sort(([left], [right]) => Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")));
        for (let index = 1; index < normalized.length; index += 1) {
            if (normalized[index - 1]?.[0] === normalized[index]?.[0]) {
                throw new CanonicalJsonError("duplicate key after NFC normalization");
            }
        }
        return `{${normalized
            .map(([key, entry]) => `${encodeString(key)}:${canonicalJson(entry)}`)
            .join(",")}}`;
    }
    throw new CanonicalJsonError("unsupported canonical JSON value");
}
function normalize(value) {
    if (!isUnicodeScalarText(value)) {
        throw new CanonicalJsonError("text contains an unpaired surrogate");
    }
    return value.normalize("NFC");
}
function isUnicodeScalarText(value) {
    for (let index = 0; index < value.length; index += 1) {
        const unit = value.charCodeAt(index);
        if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = value.charCodeAt(index + 1);
            if (!(next >= 0xdc00 && next <= 0xdfff)) {
                return false;
            }
            index += 1;
        }
        else if (unit >= 0xdc00 && unit <= 0xdfff) {
            return false;
        }
    }
    return true;
}
function encodeString(value) {
    let output = '"';
    for (const character of value) {
        switch (character) {
            case '"':
                output += '\\"';
                break;
            case "\\":
                output += "\\\\";
                break;
            case "\b":
                output += "\\b";
                break;
            case "\t":
                output += "\\t";
                break;
            case "\n":
                output += "\\n";
                break;
            case "\f":
                output += "\\f";
                break;
            case "\r":
                output += "\\r";
                break;
            default: {
                const codePoint = character.codePointAt(0);
                if (codePoint === undefined) {
                    throw new CanonicalJsonError("invalid Unicode scalar");
                }
                output +=
                    codePoint <= 0x1f
                        ? `\\u00${codePoint.toString(16).padStart(2, "0")}`
                        : character;
            }
        }
    }
    return `${output}"`;
}
//# sourceMappingURL=cjson.js.map