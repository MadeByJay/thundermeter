import { describe, expect, it } from "vitest";
import { MAX_PROFILE_NAME, profileNameProblem } from "./profile";

describe("profileNameProblem", () => {
  it("accepts ordinary names", () => {
    for (const ok of ["Gaming", "Stream (late)", "music-2", "Café"]) {
      expect(profileNameProblem(ok, ["Default"])).toBeNull();
    }
  });

  // Same refusals as persist::valid_profile_name: anything that could escape
  // profiles/, hide the file, or carry invisible characters.
  it("refuses what the engine refuses", () => {
    for (const bad of ["", " lead", "trail ", ".hidden", "../etc", "a/b", "a\\b", "tab\there", "x".repeat(MAX_PROFILE_NAME + 1)]) {
      expect(profileNameProblem(bad, [])).not.toBeNull();
    }
  });

  it("refuses an existing name rather than overwrite it", () => {
    expect(profileNameProblem("Default", ["Default", "Gaming"])).toMatch(/exists/);
  });

  it("counts characters, not UTF-16 units", () => {
    expect(profileNameProblem("é".repeat(MAX_PROFILE_NAME), [])).toBeNull();
  });
});
