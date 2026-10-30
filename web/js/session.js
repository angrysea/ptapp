// The signed-in user's profile, shared by all views.

export const session = {
  /** Profile from GET /me (role, coachId, units, …). */
  profile: null,
  /** Library cache keyed by exercise id. */
  exercises: null,
  /** An assistant's team membership (title, permissions, clients), from GET /me. */
  member: null,
  /** The coach side of the app: a head coach or one of their assistants. */
  get isCoach() { return this.profile?.role === 'coach' || this.profile?.role === 'assistant'; },
  /** The head coach: billing, invitations, settings and the team are theirs. */
  get isHeadCoach() { return this.profile?.role === 'coach'; },
  get isAssistant() { return this.profile?.role === 'assistant'; },
  /** Whether the coach side may do `perm` (team.js PERMS); the head coach may do everything. */
  can(perm) { return this.isHeadCoach || !!this.member?.perms?.includes(perm); },
  get isAdmin() { return this.profile?.role === 'admin'; },
  get userId() { return this.profile?.userId; },
};
