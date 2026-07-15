import assert from "node:assert/strict";
import test from "node:test";
import {
  conversationJoinCommand,
  conversationShareText,
  portableConversationKey,
} from "../src/shared/conversation.ts";

test("one join command carries the exact hub and stays idempotent", () => {
  const hub = "wss://parler.example/ws";
  assert.equal(portableConversationKey("ABC123", hub), `ABC123@${hub}`);
  assert.equal(
    portableConversationKey(`parler conversation ABC123@${hub}`, "ws://wrong"),
    `ABC123@${hub}`,
  );
  assert.equal(conversationJoinCommand("ABC123", hub), `parler conversation ABC123@${hub}`);
});

test("share text teaches the canonical conversation command without exposing an internal room", () => {
  const text = conversationShareText("ABC123", "ws://127.0.0.1:7070/ws");
  assert.match(text, /parler conversation ABC123@ws:\/\/127\.0\.0\.1:7070\/ws/);
  assert.doesNotMatch(text, /room\./);
  assert.doesNotMatch(text, /session join/);
});
