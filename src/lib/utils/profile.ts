/** Profile names become file names in ~/.config/thundermeter/profiles/.
 *  Mirrors `persist::valid_profile_name` in lm-engine — the engine refuses
 *  anything this rejects, so the menu checks first and can say why. */
export const MAX_PROFILE_NAME = 48;

/** Why `name` can't be used, or null when it can. `taken` is the current
 *  profile list (a new name must not overwrite one). */
export function profileNameProblem(name: string, taken: readonly string[]): string | null {
  if (name.length === 0) return "Enter a name";
  if (name.trim() !== name) return "No leading or trailing spaces";
  if ([...name].length > MAX_PROFILE_NAME) return `At most ${MAX_PROFILE_NAME} characters`;
  if (name.startsWith(".")) return "Can't start with a dot";
  // eslint-disable-next-line no-control-regex
  if (/[/\\\u0000-\u001f\u007f]/.test(name)) return "No slashes or control characters";
  if (taken.includes(name)) return "A profile with that name exists";
  return null;
}
