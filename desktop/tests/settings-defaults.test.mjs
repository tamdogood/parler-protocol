import assert from "node:assert/strict";
import test from "node:test";
import { DEFAULT_HUB_PORT, defaultSettings } from "../src/main/settings-defaults.ts";

test("fresh desktop installs use the public hub without starting a local hub", () => {
  const settings = defaultSettings("Ada");

  assert.equal(settings.connectTarget, "public");
  assert.equal(settings.autoStartHub, false);
  assert.equal(settings.hubPort, DEFAULT_HUB_PORT);
  assert.equal(settings.hubName, "Ada's Hub");
});
