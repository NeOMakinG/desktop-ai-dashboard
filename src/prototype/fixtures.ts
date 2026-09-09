export type Variant = 'studio' | 'focus' | 'canvas';
export type WorkspaceKind = 'daily' | 'inbox' | 'focus' | 'travel';
export type FixtureItem = {
  id: string;
  type: 'message' | 'meeting';
  person: string;
  initials: string;
  title: string;
  preview: string;
  body: string;
  time: string;
  end?: string;
  needsReply?: boolean;
  source: string;
};
export type Workspace = {
  revision: number;
  kind: WorkspaceKind;
  quiet: boolean;
  unread: boolean;
  notes: Record<string, string>;
  done: string[];
};
export const INITIAL_WORKSPACE: Workspace = {
  revision: 1, kind: 'daily', quiet: false, unread: false, notes: {}, done: [],
};
export const MESSAGES: FixtureItem[] = [
  { id: 'maya', type: 'message', person: 'Maya Chen', initials: 'MC', title: 'Review deck: two decisions to settle', preview: 'A smaller navigation. A clearer way to move.', body: 'The latest review deck is ready. Could we settle on the compact navigation and confirm the keyboard behavior before the interface review? I’ve left both options side by side so we can make a thoughtful call together.', time: '08:47', needsReply: true, source: 'Sample messages' },
  { id: 'jules', type: 'message', person: 'Jules Laurent', initials: 'JL', title: 'Research notes are ready', preview: 'Five conversations, a few useful discoveries.', body: 'I’ve gathered the notes from our five research sessions. Everyone found the daily view helpful, but five participants missed the save affordance. Let’s walk through the findings at 11:00 and make the next iteration feel a little more obvious.', time: '08:31', source: 'Sample messages' },
  { id: 'sofia', type: 'message', person: 'Sofia Okafor', initials: 'SO', title: 'Thursday handoff', preview: 'One last detail before we pass things along.', body: 'We’re in a good place for Thursday. Can you confirm who owns the empty states? I’ll bring the checklist to our handoff. No need to prepare a deck — a few clear notes will do.', time: '08:05', needsReply: true, source: 'Sample messages' },
];
export const MEETINGS: FixtureItem[] = [
  { id: 'sync', type: 'meeting', person: 'Maya Chen', initials: 'MC', title: 'Product sync', preview: 'A quick alignment for the day', body: 'Review the two priorities for today, confirm owners, and leave room for questions.', time: '09:30', end: '10:00', source: 'Sample calendar' },
  { id: 'research', type: 'meeting', person: 'Jules Laurent', initials: 'JL', title: 'Research playback', preview: 'The things we heard, together', body: 'Walk through five research sessions. Focus on discoverability, saving a workspace, and the first-run experience.', time: '11:00', end: '11:45', source: 'Sample calendar' },
  { id: 'review', type: 'meeting', person: 'Maya Chen & Sofia Okafor', initials: 'MC', title: 'Interface review', preview: 'Bring the save-and-restore flow', body: 'Compare Studio and Focus. Decide on navigation density and review keyboard behavior. Bring the save-and-restore flow.', time: '14:00', end: '14:45', source: 'Sample calendar' },
  { id: 'handoff', type: 'meeting', person: 'Sofia Okafor', initials: 'SO', title: 'Design handoff', preview: 'Make the next step a clear one', body: 'Confirm ownership of empty states, review the remaining questions, and write down the next steps.', time: '16:15', end: '16:35', source: 'Sample calendar' },
];
export const TRAVEL_MESSAGES: FixtureItem[] = [
  { id: 'travel-train', type: 'message', person: 'Sample Rail', initials: 'SR', title: 'Your Paris → Lyon itinerary', preview: 'Thursday, 10 September · 08:00 departure', body: 'Synthetic itinerary: depart Paris Gare de Lyon at 08:00 and arrive at Lyon Part-Dieu at 09:56. This is sample information, not a reservation or a valid ticket.', time: '08:40', needsReply: false, source: 'Sample travel messages' },
  { id: 'travel-stay', type: 'message', person: 'Maison Demo', initials: 'MD', title: 'A few details for your stay', preview: 'Check-in from 15:00 · a quiet place to land', body: 'Synthetic hotel information for the travel preview. Check-in would be from 15:00. No booking exists and no payment has been made.', time: '08:20', needsReply: true, source: 'Sample travel messages' },
];
export const TRAVEL_MEETINGS: FixtureItem[] = [
  { id: 'travel-depart', type: 'meeting', person: 'Sample itinerary', initials: 'SI', title: 'Train to Lyon', preview: 'Paris Gare de Lyon → Lyon Part-Dieu', body: 'Suggested travel block for Thursday, September 10. This fixture is not added to any calendar and does not represent a booking.', time: '08:00', end: '09:56', source: 'Sample itinerary · Thursday' },
  { id: 'travel-arrive', type: 'meeting', person: 'Sample itinerary', initials: 'SI', title: 'Time to settle in', preview: 'Leave a little breathing room', body: 'A suggested buffer before the afternoon. Nothing has been booked or added to a calendar.', time: '10:00', end: '11:00', source: 'Sample itinerary · Thursday' },
];
export const ALL_ITEMS = [...MESSAGES, ...MEETINGS, ...TRAVEL_MESSAGES, ...TRAVEL_MEETINGS];
export const WORKSPACE_COPY: Record<WorkspaceKind, { title: string; subtitle: string; brief: string; note: string; label: string }> = {
  daily: { title: 'A calmer kind of day.', subtitle: 'The important things, with room to breathe.', brief: 'Two messages need a reply. Your interface review starts at 14:00.', note: 'Compare Studio and Focus; bring the save-and-restore flow.', label: 'Daily workspace' },
  inbox: { title: 'An inbox with intention.', subtitle: 'A little less sorting. A little more clarity.', brief: 'Two conversations need your attention. Everything else can wait.', note: 'Start with Maya’s review decisions, then confirm the handoff with Sofia.', label: 'Inbox essentials' },
  focus: { title: 'Make room for deep work.', subtitle: 'A quieter workspace for what matters next.', brief: 'Protect a little time to prepare. Your interface review starts at 14:00.', note: 'Compare Studio and Focus; bring the save-and-restore flow.', label: 'Focus space' },
  travel: { title: 'A little space to explore.', subtitle: 'Your sample Lyon trip, thoughtfully arranged.', brief: 'Thursday in Lyon. Your sample itinerary leaves room to settle in.', note: 'Keep your itinerary close and leave a little time between arrival and your next stop. No bookings have been made.', label: 'Lyon, thoughtfully' },
};
export function classifyPrompt(prompt: string): { kind: WorkspaceKind; supported: boolean } {
  if (/travel|trip|lyon|train|flight|hotel|itinerary/i.test(prompt)) return { kind: 'travel', supported: true };
  if (/focus|quiet|prepare|preparation|protect|deep work/i.test(prompt)) return { kind: 'focus', supported: true };
  if (/inbox|email|mail|unread|messages|reply/i.test(prompt)) return { kind: 'inbox', supported: true };
  if (/day|daily|workspace|calendar|morning|general/i.test(prompt)) return { kind: 'daily', supported: true };
  return { kind: 'daily', supported: false };
}
