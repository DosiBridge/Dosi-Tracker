import { describe, it, expect } from "vitest";
import { brand, greeting } from "./brand";

describe("brand palette", () => {
  it("exposes hex accents used by charts/inline styles", () => {
    expect(brand.primary).toMatch(/^#[0-9a-f]{6}$/i);
    expect(Object.keys(brand)).toContain("danger");
  });
});

describe("greeting", () => {
  it("uses only the first name", () => {
    expect(greeting("Ada Lovelace", 9)).toBe("Good morning, Ada");
  });
  it.each([
    [0, "Good morning, Sam"],
    [11, "Good morning, Sam"],
    [12, "Good afternoon, Sam"],
    [16, "Good afternoon, Sam"],
    [17, "Good evening, Sam"],
    [23, "Good evening, Sam"],
  ])("hour %i -> %s", (hour, expected) => {
    expect(greeting("Sam", hour)).toBe(expected);
  });
});
