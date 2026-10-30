// #/help: the user manual, one per role, opened from the ? in the header.
// Coaches and clients see their own; admins see all three (they support
// both). Plain data below, so the wording is easy to keep current.

import { go } from '../router.js';
import { session } from '../session.js';
import { $app, h, mount, subtabs } from '../ui.js';

// A section's body is a list of: a string (a paragraph), { steps: [] } (a
// numbered how-to) or { list: [] } (bullets).
const MANUALS = {
  admin: {
    title: 'Admin manual',
    intro: 'As an admin you decide who coaches on the platform and look after coach accounts. Your tabs are Coaches, Directory and Profile.',
    sections: [
      {
        title: 'Invite a coach',
        body: [
          'Coaches join by invitation only. An invitation is tied to one email address and works once.',
          { steps: [
            'On Coaches, tap + Invite coach.',
            'Enter the coach’s email and tap Send invitation. The email comes from invites@gymtxcoach.com.',
            'The coach opens the link, which fills in and locks their email on the sign-up page, then chooses a password.',
          ] },
          'Pending invitations are listed under Pending invitations. Use Resend email if it didn’t arrive, Link to copy the link and send it yourself, or Revoke to cancel it.',
        ],
      },
      {
        title: 'Coach requests',
        body: [
          'Coaches without an invitation can ask to join from the form on the landing page. Requests appear under Coach requests on the Coaches tab, and you get an email for each.',
          { list: [
            'Approve emails the coach the same single-use invitation as + Invite coach.',
            'Deny closes the request without emailing.',
            'Turn on auto-approve to approve every request as it arrives.',
          ] },
          'The form has a human check and a per-sender limit, so you shouldn’t see bulk spam.',
        ],
      },
      {
        title: 'Look after a coach',
        body: [
          'Tap a coach on the Coaches tab to open their details.',
          { list: [
            'Business details (private) and Certifications: what the coach entered on their profile.',
            'Public profile: hide the coach from the public directory, or show them again.',
            'Coaching time (last 4 weeks): minutes the coach spent on client pages, per client per week.',
            'Clients: everyone the coach trains.',
          ] },
        ],
      },
      {
        title: 'Move a client to another coach',
        body: [
          { steps: [
            'Open the client’s current coach on the Coaches tab.',
            'Under Clients, tap Move… beside the client.',
            'Pick the new coach and confirm.',
          ] },
          'The client’s history moves with them: workouts, logs, body data, photos, videos and messages. The new coach should assign a program if the old one doesn’t carry over.',
        ],
      },
      {
        title: 'Suspend or delete a coach',
        body: [
          'Suspend a coach to lock them out without losing anything: they can’t sign in and their calendar feeds stop, but their clients can still log workouts. Tap the same button again to lift the suspension.',
          'Delete coach account removes the coach permanently. Move their clients to another coach first, or those clients lose their coach too. This can’t be undone.',
        ],
      },
      {
        title: 'Support coaches and clients',
        body: [
          'The Coach manual and Client manual tabs above are what coaches and clients see. Use them to answer questions or walk someone through a task.',
          'The Directory tab shows the public coach directory exactly as visitors see it.',
        ],
      },
    ],
  },

  coach: {
    title: 'Coach manual',
    intro: 'Your tabs are Clients, Messages, Videos, Programs, Exercises and Profile. Most day-to-day work starts on Clients.',
    sections: [
      {
        title: 'Get set up',
        body: [
          { steps: [
            'Complete your coach profile when you first sign in: photo, bio, specialties, certifications and business details. Edit it later from Profile → Edit coach profile.',
            'Set your business name on Profile. It brands the app header for you and your clients.',
            'Use View my public page to check how you appear in the directory.',
            'On Profile, set your hours, notifications, alert settings, check-in questions and intake questions.',
          ] },
          'Install the app on your phone’s home screen (on iPhone: Safari → Share → Add to Home Screen). Push notifications on iPhone need the installed app.',
        ],
      },
      {
        title: 'Add clients',
        body: [
          { list: [
            'Invite by email: on Clients, tap + Invite, enter the client’s email, and send. Pending invites show on Clients with Resend email, Link and Revoke.',
            'Your sign-up link: share 🔗 Your sign-up link, or put the website button (Copy button HTML) on your site. People who use it land in your request queue.',
            'Requests: approve to send the invitation, or deny. Turn on auto-approve to accept everyone.',
          ] },
          'New clients complete the intake questionnaire and the health screening & waiver. The roster flags anyone who needs clearance or hasn’t signed.',
        ],
      },
      {
        title: 'The Clients tab',
        body: [
          'Every client shows 28-day adherence and flags: Pain, Needs clearance, No waiver, No program and Missed. Alerts and progression suggestions appear at the top.',
          'Tap a client to open their pages:',
          { list: [
            'Overview: adherence, missed dates, weekly volume, strength trends, maxes, records, habits, nutrition targets, credits and the starting point.',
            'Calendar: their scheduled workouts. Tap a day to add or edit its workout.',
            'Logs: what they did, set by set, with their ratings and comments.',
            'Check-ins: weekly check-in answers with a 4-week view.',
            'Body: weight, tape, skinfolds, body fat and progress photos.',
            'Intake: their questionnaire, screening and waiver.',
            'Messages: your thread with this client.',
          ] },
          'Use 📄 Progress report on a client to print or save a PDF of their progress.',
        ],
      },
      {
        title: 'Build programs',
        body: [
          'Programs are multi-week calendars of workouts. Start from scratch, from a template, or from Starter programs (strength benchmark, 12-week strength, kettlebell & dumbbell, bodyweight, bodybuilding splits, strength for runners).',
          { steps: [
            'On Programs, tap + New or open a starter and tap Use this program.',
            'Tap a day to add a workout, then add blocks: straight sets, superset, circuit, AMRAP, EMOM, interval, steady state, flow, ramp-up or for time.',
            'Add exercises and set sets, reps, load, tempo, rest, RPE, RIR and notes. Load can be a % of 1RM; give a fallback weight for clients with no max.',
            'Copy a week, move or copy a workout, or Duplicate the whole program to save time.',
            'Optionally add a progression rule to an exercise. When a client meets it, you get a suggestion to accept or dismiss.',
          ] },
          'Assign a program from a client’s Overview (Assign a program). Each client gets their own copy, so you can tailor it; edits re-write that client’s upcoming workouts only.',
        ],
      },
      {
        title: 'Exercises',
        body: [
          'The library has built-in strength, kettlebell, bodyweight, cardio and mobility exercises. Add your own with cues, tags, filming tips, and a demo link or an uploaded demo video.',
        ],
      },
      {
        title: 'Maxes and records',
        body: [
          'Record a client’s one-rep maxes on their Overview (Record or change a max), or have them do the strength benchmark program. A finished workout that beats a max raises it.',
          'Personal records (rep maxes and estimated 1RMs) are found when the client logs. Remove one that came from a typo.',
        ],
      },
      {
        title: 'Review set videos',
        body: [
          'Clients film sets from their workout. New videos appear on the Videos tab, oldest first, with a count badge.',
          { steps: [
            'Open a video. Use slow motion and frame stepping (, and . keys on a computer) to check technique.',
            'Reply in writing, or tap ● Record reply to record a video reply: talk over the clip, draw on it (pen, line, arrow, circle, angle, timer), compare side by side or overlaid, and include your webcam in a corner.',
            'Tap ✓ Mark reviewed. Reviewed videos are deleted after 180 days.',
          ] },
        ],
      },
      {
        title: 'Messages',
        body: [
          'The Messages tab is your inbox across all clients, updated live, with unread badges. Send text, photos (📷 Photo) and voice notes (🎤 Voice), and link a message to a workout, set or video.',
          'Save replies you send often as Snippets, then type / in a message to insert one.',
          'Set reminders and automated messages under Profile → Reminders & automated messages. They go out at each client’s local time.',
        ],
      },
      {
        title: 'Nutrition and habits',
        body: [
          { list: [
            'Set daily calorie and macro targets on a client’s Overview. Clients see meters; you see 7-day averages.',
            'Clients log meals as plain lines (“chicken 12oz”). Tap Estimate to fill in calories from USDA food search, your saved staples, or by hand.',
            'Clients can import a Cronometer or MyFitnessPal diary.',
            'Add habits for a client (Add a habit); they tick them off daily and build streaks.',
            'Recipes and meal plans: keep recipes in books shared with all or chosen clients, import them from text, a PDF or a web page, and assign meal plans.',
          ] },
        ],
      },
      {
        title: 'Sessions and plans',
        body: [
          { list: [
            'Set When clients can book on 📅 Schedule. Clients book open slots; you can book any time.',
            'Add your sessions to your phone with the private calendar feed (Profile → Add to calendar).',
            'Plans & credits on Profile: create plans (credits per service, or unlimited, for a number of days, with a price). Grant a plan or credits on a client’s Overview.',
            'Turn on Require credits to stop clients sending messages, photos or videos, or booking, without a credit.',
          ] },
          'Your coaching time (Profile) shows minutes per client per week.',
        ],
      },
    ],
  },

  client: {
    title: 'Client manual',
    intro: 'Your tabs are Today, Progress, Coach and Profile. Everything you log works offline and syncs when you’re back online.',
    sections: [
      {
        title: 'Get started',
        body: [
          { steps: [
            'Open your coach’s invitation email and create your password. (No invitation? Find a coach under Browse coaches and send a request.)',
            'Agree to the data agreement.',
            'Complete the intake questionnaire and the health screening & waiver from Profile. Your coach can’t clear you to train until both are done.',
            'Install the app: on iPhone, Safari → Share → Add to Home Screen. On Android, use Install app in the browser menu.',
            'On Profile, turn on Notifications and pick kg/lb units.',
          ] },
        ],
      },
      {
        title: 'Do today’s workout',
        body: [
          'Today shows your workout, habits and food journal. Rest days say so.',
          { steps: [
            'Tap the workout to open it. Each set is pre-filled with what your coach prescribed.',
            'Change reps, load or RPE if you did something different, and tick the set.',
            'Use ⏱ Timer for rest, EMOM, AMRAP and interval work. It beeps and keeps the screen on.',
            'Tap Finish workout, answer How was this workout? (effort, energy, sleep, soreness, any pain), and add a comment if you like.',
          ] },
          'Can’t train? Use Skip workout and tell your coach why. New personal records are celebrated when you finish.',
        ],
      },
      {
        title: 'Film a set',
        body: [
          { steps: [
            'In a workout, tap 🎥 Add video on the set. Read the filming tips for that exercise.',
            'Tap ● Record with camera (up to 60 seconds), or choose a clip from your phone.',
            'Trim it to the part that matters and send.',
          ] },
          'Uploads carry on after a lost connection or closing the app. Your coach’s written or video reply appears with the set and in your messages.',
        ],
      },
      {
        title: 'Track progress',
        body: [
          'The Progress tab holds your body data and results:',
          { list: [
            'Log weight to see a 7-day trend and weekly averages.',
            'Tape measurements and skinfolds (with a site guide) give body fat %, lean mass and fat mass.',
            'Progress photos are private to you and your coach; compare two dates side by side.',
            'Maxes and personal records.',
            '📄 Report prints or saves a PDF of your progress.',
          ] },
          'Fill in the 📝 Weekly check-in when it’s due; your coach reads every answer.',
        ],
      },
      {
        title: 'Message your coach',
        body: [
          'The Coach tab is your thread with your coach, live. Send text, a 📷 Photo or a 🎤 Voice note. Your coach’s hours show at the top.',
          'From a workout, 💬 Ask your coach about this workout links your message to it.',
        ],
      },
      {
        title: 'Food and habits',
        body: [
          { list: [
            'On Today, tap + Log a meal and write what you ate, one item per line with amounts (“oats 80g”, “chicken 12oz”). Your coach estimates the calories.',
            'If your coach sets targets, meters show calories and macros for the day.',
            'Already use Cronometer or MyFitnessPal? Use Import from Cronometer or MyFitnessPal under your food journal.',
            'Tick off your daily habits on Today to build streaks.',
            'Browse recipes your coach shares, save your own, and follow an assigned meal plan.',
          ] },
        ],
      },
      {
        title: 'Book sessions',
        body: [
          'Tap Book a session (under your coach’s hours on the Coach tab) or your next session on Today to book an open slot in your time zone, or cancel one. Add bookings to your phone with Profile → Add to calendar.',
          'If your coach uses plans, Profile shows Your plan and the credits you have left. Some things, like booking or sending a video, may need a credit.',
        ],
      },
      {
        title: 'Your account',
        body: [
          { list: [
            'Change password on Profile.',
            'Download my data gives you a copy of everything you’ve logged.',
            'Delete my account removes your account and data permanently.',
            'Profile shows anything still waiting to sync. Don’t sign out with changes waiting, or they’re lost.',
          ] },
        ],
      },
    ],
  },
};

const ROLES = [['admin', 'Admin manual'], ['coach', 'Coach manual'], ['client', 'Client manual']];

function block(item) {
  if (typeof item === 'string') return h('p', {}, item);
  if (item.steps) return h('ol', {}, item.steps.map((s) => h('li', {}, s)));
  return h('ul', {}, item.list.map((s) => h('li', {}, s)));
}

/** Coaches and clients get their own manual; admins can open any of the three. */
export function renderManual(role) {
  const own = session.profile.role;
  const pick = own === 'admin' && MANUALS[role] ? role : own;
  const m = MANUALS[pick];
  mount($app(),
    h('h1', {}, 'User manual'),
    own === 'admin' && subtabs(ROLES, pick, (r) => go(`/help/${r}`)),
    h('div', { class: 'manual' },
      own === 'admin' && h('h2', {}, m.title),
      h('p', { class: 'muted' }, m.intro),
      m.sections.map((s, i) => h('details', { class: 'card', open: i === 0 },
        h('summary', {}, s.title),
        s.body.map(block)))));
}
