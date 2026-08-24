import { createContext, useContext, type RefObject } from "react";

export const ChatScrollContext = createContext<RefObject<HTMLDivElement | null> | null>(null);

export function useChatScrollRef() {
  return useContext(ChatScrollContext);
}
