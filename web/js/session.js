// The signed-in user's profile, shared by all views.

export const session = {
  /** Profile from GET /me (role, coachId, units, …). */
  profile: null,
  /** Library cache keyed by exercise id. */
  exercises: null,
  get isCoach() { return this.profile?.role === 'coach'; },
  get userId() { return this.profile?.userId; },
};
