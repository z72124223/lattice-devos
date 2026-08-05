export type CanonicalValue = null | boolean | string | ReadonlyArray<CanonicalValue> | CanonicalObject;
export interface CanonicalObject {
    readonly [key: string]: CanonicalValue;
}
export declare class CanonicalJsonError extends Error {
    constructor(message: string);
}
export declare function canonicalJson(value: CanonicalValue): string;
//# sourceMappingURL=cjson.d.ts.map