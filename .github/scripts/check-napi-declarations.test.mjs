import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

import { findDeclarationMismatches } from "./check-napi-declarations.mjs";

const generatedDeclaration = `
export declare class SourceMap {
  source(index: number): string | null
  name(index: number): string | null
}
`;

describe("findDeclarationMismatches", () => {
  it("accepts matching selected method signatures", () => {
    const mismatches = findDeclarationMismatches(generatedDeclaration, generatedDeclaration);
    assert.deepEqual(mismatches, []);
  });

  it("reports a mismatching selected method signature", () => {
    const publicDeclaration = generatedDeclaration.replace(
      "source(index: number): string | null",
      "source(index: number): string",
    );

    const mismatches = findDeclarationMismatches(generatedDeclaration, publicDeclaration);

    assert.deepEqual(mismatches, [
      "source: generated returns string | null, public declaration returns string",
    ]);
  });

  it("keeps the public declaration aligned with generated nullability", async () => {
    const publicDeclaration = await readFile(
      new URL("../../packages/sourcemap/index.d.ts", import.meta.url),
      "utf8",
    );

    assert.deepEqual(findDeclarationMismatches(generatedDeclaration, publicDeclaration), []);
  });
});
