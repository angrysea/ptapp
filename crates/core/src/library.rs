//! Built-in exercise library (WB-1). Shipped inside the binary so it costs no
//! storage or reads; coaches add custom exercises on top in DynamoDB.

use crate::model::{CameraAngle, Exercise, Filming, Modality};

struct Seed {
    id: &'static str,
    name: &'static str,
    muscles: &'static [&'static str],
    equipment: &'static [&'static str],
    cues: &'static [&'static str],
}

const SEEDS: &[Seed] = &[
    Seed { id: "back-squat", name: "Back Squat", muscles: &["quads", "glutes", "adductors"], equipment: &["barbell", "rack"], cues: &["Brace before you descend", "Knees track over toes", "Hit depth: hip crease below knee"] },
    Seed { id: "front-squat", name: "Front Squat", muscles: &["quads", "glutes", "upper back"], equipment: &["barbell", "rack"], cues: &["Elbows high", "Stay upright", "Sit between your heels"] },
    Seed { id: "goblet-squat", name: "Goblet Squat", muscles: &["quads", "glutes"], equipment: &["dumbbell", "kettlebell"], cues: &["Hold the weight at your chest", "Elbows inside knees at the bottom"] },
    Seed { id: "deadlift", name: "Deadlift", muscles: &["hamstrings", "glutes", "back"], equipment: &["barbell"], cues: &["Bar over mid-foot", "Pull the slack out", "Push the floor away", "Lock out with glutes, not lower back"] },
    Seed { id: "romanian-deadlift", name: "Romanian Deadlift", muscles: &["hamstrings", "glutes"], equipment: &["barbell", "dumbbell"], cues: &["Soft knees", "Push hips back", "Bar stays close to legs"] },
    Seed { id: "hip-thrust", name: "Hip Thrust", muscles: &["glutes", "hamstrings"], equipment: &["barbell", "bench"], cues: &["Chin tucked", "Shins vertical at the top", "Squeeze glutes at lockout"] },
    Seed { id: "bench-press", name: "Bench Press", muscles: &["chest", "triceps", "front delts"], equipment: &["barbell", "bench", "rack"], cues: &["Shoulder blades pinched", "Feet planted", "Touch lower chest, press back over shoulders"] },
    Seed { id: "incline-db-press", name: "Incline Dumbbell Press", muscles: &["upper chest", "front delts", "triceps"], equipment: &["dumbbell", "bench"], cues: &["30-45 degree bench", "Elbows about 45 degrees from torso"] },
    Seed { id: "overhead-press", name: "Overhead Press", muscles: &["shoulders", "triceps"], equipment: &["barbell"], cues: &["Squeeze glutes", "Head through at the top", "Bar path straight up"] },
    Seed { id: "push-up", name: "Push-up", muscles: &["chest", "triceps", "core"], equipment: &["bodyweight"], cues: &["Body in a straight line", "Chest to the floor"] },
    Seed { id: "dip", name: "Dip", muscles: &["chest", "triceps"], equipment: &["dip bars"], cues: &["Slight forward lean", "Shoulders down, away from ears"] },
    Seed { id: "pull-up", name: "Pull-up", muscles: &["lats", "biceps"], equipment: &["pull-up bar"], cues: &["Start from a dead hang", "Drive elbows to ribs", "Chin over bar"] },
    Seed { id: "lat-pulldown", name: "Lat Pulldown", muscles: &["lats", "biceps"], equipment: &["cable"], cues: &["Lean back slightly", "Pull to upper chest"] },
    Seed { id: "barbell-row", name: "Barbell Row", muscles: &["upper back", "lats"], equipment: &["barbell"], cues: &["Hinge to about 45 degrees", "Pull to lower ribs", "No torso swing"] },
    Seed { id: "one-arm-db-row", name: "One-arm Dumbbell Row", muscles: &["lats", "upper back"], equipment: &["dumbbell", "bench"], cues: &["Flat back", "Pull elbow toward hip"] },
    Seed { id: "seated-cable-row", name: "Seated Cable Row", muscles: &["upper back", "lats"], equipment: &["cable"], cues: &["Tall chest", "Squeeze shoulder blades"] },
    Seed { id: "face-pull", name: "Face Pull", muscles: &["rear delts", "upper back"], equipment: &["cable", "band"], cues: &["Pull to eye level", "Rotate hands out at the end"] },
    Seed { id: "lateral-raise", name: "Lateral Raise", muscles: &["side delts"], equipment: &["dumbbell", "cable"], cues: &["Lead with elbows", "Stop at shoulder height"] },
    Seed { id: "bicep-curl", name: "Biceps Curl", muscles: &["biceps"], equipment: &["dumbbell", "barbell", "cable"], cues: &["Elbows pinned", "Control the lowering"] },
    Seed { id: "triceps-pushdown", name: "Triceps Pushdown", muscles: &["triceps"], equipment: &["cable"], cues: &["Elbows at sides", "Full lockout"] },
    Seed { id: "walking-lunge", name: "Walking Lunge", muscles: &["quads", "glutes"], equipment: &["bodyweight", "dumbbell"], cues: &["Long stride", "Back knee kisses the floor", "Torso tall"] },
    Seed { id: "bulgarian-split-squat", name: "Bulgarian Split Squat", muscles: &["quads", "glutes"], equipment: &["dumbbell", "bench"], cues: &["Most weight on the front foot", "Control the descent"] },
    Seed { id: "step-up", name: "Step-up", muscles: &["quads", "glutes"], equipment: &["box", "dumbbell"], cues: &["Drive through the whole front foot", "No push from the back leg"] },
    Seed { id: "leg-press", name: "Leg Press", muscles: &["quads", "glutes"], equipment: &["machine"], cues: &["Lower back stays on the pad", "Don't lock knees hard"] },
    Seed { id: "leg-curl", name: "Leg Curl", muscles: &["hamstrings"], equipment: &["machine"], cues: &["Hips down", "Slow eccentric"] },
    Seed { id: "calf-raise", name: "Calf Raise", muscles: &["calves"], equipment: &["machine", "bodyweight"], cues: &["Full stretch at the bottom", "Pause at the top"] },
    Seed { id: "kettlebell-swing", name: "Kettlebell Swing", muscles: &["glutes", "hamstrings"], equipment: &["kettlebell"], cues: &["Hike it back", "Snap the hips", "Bell floats to chest height"] },
    Seed { id: "plank", name: "Plank", muscles: &["core"], equipment: &["bodyweight"], cues: &["Squeeze glutes", "Ribs down", "Straight line head to heel"] },
    Seed { id: "dead-bug", name: "Dead Bug", muscles: &["core"], equipment: &["bodyweight"], cues: &["Lower back pressed down", "Exhale as you extend"] },
    Seed { id: "farmers-carry", name: "Farmer's Carry", muscles: &["grip", "core", "traps"], equipment: &["dumbbell", "kettlebell"], cues: &["Tall posture", "Short quick steps"] },
    Seed { id: "burpee", name: "Burpee", muscles: &["full body"], equipment: &["bodyweight"], cues: &["Chest to floor", "Jump and clap overhead"] },
    Seed { id: "rowing-erg", name: "Rowing (erg)", muscles: &["full body", "conditioning"], equipment: &["rower"], cues: &["Legs, body, arms; arms, body, legs"] },
    Seed { id: "assault-bike", name: "Air Bike", muscles: &["conditioning"], equipment: &["air bike"], cues: &["Push and pull with arms"] },
    Seed { id: "jump-rope", name: "Jump Rope", muscles: &["calves", "conditioning"], equipment: &["jump rope"], cues: &["Stay on the balls of your feet", "Wrists turn the rope"] },
    Seed { id: "treadmill-run", name: "Treadmill Run", muscles: &["conditioning"], equipment: &["treadmill"], cues: &["Run tall, eyes forward", "Short, quick steps", "Don't hold the rails"] },
    Seed { id: "incline-walk", name: "Incline Walk", muscles: &["conditioning", "glutes", "calves"], equipment: &["treadmill"], cues: &["Hands off the rails", "Push through the whole foot"] },
    Seed { id: "outdoor-run", name: "Run (outdoors)", muscles: &["conditioning"], equipment: &["bodyweight"], cues: &["Easy pace: you can still talk", "Land under your hips"] },
    Seed { id: "brisk-walk", name: "Brisk Walk", muscles: &["conditioning"], equipment: &["bodyweight"], cues: &["Arms swing", "Pace where talking takes effort"] },
    Seed { id: "stationary-bike", name: "Stationary Bike", muscles: &["conditioning", "quads"], equipment: &["bike"], cues: &["Seat at hip height when standing", "Slight bend in the knee at the bottom"] },
    Seed { id: "elliptical", name: "Elliptical", muscles: &["conditioning"], equipment: &["elliptical"], cues: &["Stand tall", "Push and pull the handles"] },
    Seed { id: "stair-climber", name: "Stair Climber", muscles: &["conditioning", "glutes", "quads"], equipment: &["stair climber"], cues: &["Whole foot on each step", "Light hands on the rails"] },
    Seed { id: "ski-erg", name: "Ski Erg", muscles: &["full body", "conditioning"], equipment: &["ski erg"], cues: &["Hinge and pull with lats", "Arms finish past the hips"] },
    Seed { id: "swim", name: "Swim", muscles: &["full body", "conditioning"], equipment: &["pool"], cues: &["Long, relaxed strokes", "Breathe every 2-3 strokes"] },
    Seed { id: "worlds-greatest-stretch", name: "World's Greatest Stretch", muscles: &["hips", "thoracic spine", "hamstrings"], equipment: &["bodyweight"], cues: &["Elbow toward the instep", "Rotate and reach to the ceiling"] },
    Seed { id: "cat-cow", name: "Cat-Cow", muscles: &["spine"], equipment: &["bodyweight"], cues: &["Move one segment at a time", "Breathe with the movement"] },
    Seed { id: "leg-swing", name: "Leg Swings", muscles: &["hips", "hamstrings"], equipment: &["bodyweight"], cues: &["Hold something for balance", "Swing a little higher each rep"] },
    Seed { id: "band-pull-apart", name: "Band Pull-apart", muscles: &["rear delts", "upper back"], equipment: &["band"], cues: &["Arms straight", "Squeeze shoulder blades together"] },
];

pub fn builtin_exercises() -> Vec<Exercise> {
    SEEDS
        .iter()
        .map(|s| Exercise {
            id: s.id.to_string(),
            name: s.name.to_string(),
            muscle_groups: s.muscles.iter().map(|m| m.to_string()).collect(),
            equipment: s.equipment.iter().map(|m| m.to_string()).collect(),
            cues: s.cues.iter().map(|m| m.to_string()).collect(),
            demo_url: None,
            demo_key: None,
            pending_demo_key: None,
            custom: false,
            modality: modality(s.id),
            filming: filming(s.id),
        })
        .collect()
}

fn modality(id: &str) -> Modality {
    match id {
        "rowing-erg" | "assault-bike" | "jump-rope" | "treadmill-run" | "incline-walk" | "outdoor-run" | "brisk-walk"
        | "stationary-bike" | "elliptical" | "stair-climber" | "ski-erg" | "swim" => Modality::Cardio,
        "worlds-greatest-stretch" | "cat-cow" | "leg-swing" | "band-pull-apart" => Modality::Mobility,
        _ => Modality::Strength,
    }
}

/// Default filming guidance for the built-in lifts worth reviewing on video.
fn filming(id: &str) -> Option<Filming> {
    let (angle, distance_m, notes) = match id {
        "back-squat" | "front-squat" | "goblet-squat" => (CameraAngle::Diagonal, 3.0, "Phone at hip height so depth and knee travel both show."),
        "deadlift" | "romanian-deadlift" => (CameraAngle::Side, 3.0, "Phone at hip height, level with the bar, so bar path and back angle show."),
        "hip-thrust" => (CameraAngle::Side, 2.5, "Phone at bench height, level with your hips."),
        "bench-press" | "incline-db-press" => (CameraAngle::Diagonal, 2.0, "Phone by your feet, a little to one side, so the bar path shows."),
        "overhead-press" => (CameraAngle::Side, 3.0, "Phone at chest height so the bar path and lockout show."),
        "barbell-row" | "one-arm-db-row" => (CameraAngle::Side, 2.5, "Phone at hip height so your back angle shows."),
        "pull-up" | "lat-pulldown" | "seated-cable-row" => (CameraAngle::Side, 2.5, "Whole movement in frame, top to bottom."),
        "push-up" | "dip" => (CameraAngle::Side, 2.0, "Phone low, at shoulder height, so body line and depth show."),
        "walking-lunge" | "bulgarian-split-squat" | "step-up" => (CameraAngle::Front, 3.0, "Phone at knee height so knee tracking shows."),
        "kettlebell-swing" => (CameraAngle::Side, 3.0, "Phone at hip height so the hinge shows."),
        "lateral-raise" => (CameraAngle::Front, 2.5, "Phone at shoulder height."),
        _ => return None,
    };
    Some(Filming { angle, distance_m: Some(distance_m), notes: notes.to_string() })
}

pub fn is_builtin(id: &str) -> bool {
    SEEDS.iter().any(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = super::SEEDS.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), super::SEEDS.len());
    }

    #[test]
    fn main_lifts_have_filming_guidance() {
        let all = super::builtin_exercises();
        let get = |id: &str| all.iter().find(|e| e.id == id).unwrap().filming.clone();
        for id in ["back-squat", "deadlift", "bench-press", "overhead-press"] {
            let f = get(id).unwrap_or_else(|| panic!("{id} has no filming guidance"));
            assert!(f.validate().is_ok());
        }
        assert!(get("plank").is_none());
    }

    #[test]
    fn cardio_machines_are_cardio() {
        let all = super::builtin_exercises();
        let modality = |id: &str| all.iter().find(|e| e.id == id).unwrap().modality;
        assert_eq!(modality("rowing-erg"), super::Modality::Cardio);
        assert_eq!(modality("incline-walk"), super::Modality::Cardio);
        assert_eq!(modality("back-squat"), super::Modality::Strength);
    }
}
