/** Normalize a bare key, portable descriptor, or already-rendered command to one KEY@HUB value. */
export function portableConversationKey(value: string, hub: string): string {
  const descriptor = value.trim().replace(/^parler\s+conversation\s+/, "");
  return descriptor.includes("@") ? descriptor : `${descriptor}@${hub}`;
}

/** The one canonical command a visible agent runs to join. */
export function conversationJoinCommand(value: string, hub: string): string {
  return `parler conversation ${portableConversationKey(value, hub)}`;
}

/** User-facing payload for macOS Share, clipboard fallbacks, Messages, and Mail. */
export function conversationShareText(value: string, hub: string): string {
  return `Join my Parler Protocol conversation:\n\n${conversationJoinCommand(value, hub)}`;
}
