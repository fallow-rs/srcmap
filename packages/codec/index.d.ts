/**
 * Decode a VLQ-encoded source map mappings string.
 *
 * Returns an array of lines, each containing an array of segments.
 * Each segment is an array of 1, 4, or 5 numbers.
 *
 * Compatible with `@jridgewell/sourcemap-codec` decode().
 */
export declare function decode(mappings: string): number[][][]

/**
 * Encode decoded source map mappings back into a VLQ string.
 *
 * Takes an array of lines, each containing an array of segments.
 * Each segment should be an array of 1, 4, or 5 numbers.
 *
 * Compatible with `@jridgewell/sourcemap-codec` encode().
 */
export declare function encode(mappings: number[][][]): string
