// Explanations of effort ratings (RPE, RIR) behind the ⓘ buttons. Coaches
// set them as targets in programs; clients rate how hard each set actually
// felt when they log it (RPE only), plus the whole session.

import { h, infoButton } from './ui.js';

const scale = () => h('table', { class: 'scale' },
  h('tr', {}, h('th', {}, 'RPE'), h('th', {}, 'How it felt')),
  [
    ['10', 'Maximum: no more reps possible'],
    ['9', 'Could have done 1 more rep'],
    ['8', 'Could have done 2 more reps'],
    ['7', 'Could have done 3 more reps, bar still moving fast'],
    ['5-6', 'Could have done 4-6 more reps'],
    ['1-4', 'Very light, warm-up effort'],
  ].map(([n, t]) => h('tr', {}, h('td', {}, n), h('td', {}, t))));

/** For the coach writing a program: the RPE target. */
export const rpeTargetInfo = () => infoButton('RPE',
  h('p', {}, 'RPE (rate of perceived exertion) is how hard a set feels, on a 1-10 scale.'),
  scale(),
  h('p', {}, 'Here it’s the target: the client picks a weight that makes each set feel about this hard, so the load adjusts to how they feel that day. When they log the workout they rate how hard each set actually felt, and you see both.'),
  h('p', { class: 'muted small' }, 'Use RPE or RIR for an exercise, not usually both: they describe the same thing (RPE 8 = 2 RIR).'));

/** For the coach writing a program: the RIR target. */
export const rirTargetInfo = () => infoButton('RIR',
  h('p', {}, 'RIR (reps in reserve) is how many more good reps the client could still do at the end of a set.'),
  h('p', {}, 'Here it’s the target: 2 RIR means stop each set with about two good reps left in the tank. 0 RIR means to failure.'),
  h('p', {}, 'It’s the same idea as RPE counted the other way: 0 RIR = RPE 10, 1 RIR = RPE 9, 2 RIR = RPE 8, 3 RIR = RPE 7. Clients find RIR easier to judge; clients log their sets as RPE.'),
  h('p', { class: 'muted small' }, 'Use RPE or RIR for an exercise, not usually both.'));

/** For the client logging sets: rate how hard each set felt. */
export const rpeLogInfo = () => infoButton('RPE',
  h('p', {}, 'After each set, rate how hard it felt from 1 to 10. Think about how many more good reps you could have done.'),
  scale(),
  h('p', {}, 'If your coach set a target (for example “RPE 8” or “2 RIR” above your sets), choose a weight that makes your sets feel about that hard. 2 RIR means stop with about two reps left, the same as RPE 8.'),
  h('p', { class: 'muted small' }, 'Not sure? Leave it blank. An honest guess is more useful to your coach than a perfect number.'));

/** For the client's post-workout check: the whole session. */
export const sessionRpeInfo = () => infoButton('Session RPE',
  h('p', {}, 'How hard the whole workout felt, from 1 (very easy) to 10 (as hard as you could possibly go).'),
  h('p', {}, 'Rate it about 30 minutes after you finish if you can, once your breathing has settled. Your coach uses it to see how hard training is overall and to spot when you need an easier week.'));

/** The overall workout rating the client gives (WorkoutLog.rating). */
export const RATINGS = [[1, 'Bad', '😞'], [2, 'Poor', '🙁'], [3, 'OK', '😐'], [4, 'Good', '🙂'], [5, 'Great', '😄']];
