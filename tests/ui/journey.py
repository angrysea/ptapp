import os, re, struct, sys, time, datetime, zlib
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

BASE = os.environ.get("BASE_URL", "http://localhost:8080/")
OUT = Path(__file__).parent / "shots"
OUT.mkdir(exist_ok=True)
errors = []
RUN = str(int(time.time()))

def tiny_png(w=8, h=8):
    """A valid w×h solid-colour PNG, for sending a photo."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    row = bytes([0]) + bytes([0x20, 0x80, 0x60]) * w  # filter byte, then RGB pixels
    signature = bytes([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    return (signature + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(row * h)) + chunk(b"IEND", b""))

def tiny_pdf(lines):
    """A one-page PDF with each of `lines` on its own line (real text, so pdf.js reads it)."""
    esc = lambda t: t.replace("\\", "\\\\").replace("(", "\(").replace(")", "\)")
    text = "BT /F1 12 Tf 50 750 Td 16 TL " + " ".join(f"({esc(l)}) Tj T*" for l in lines) + " ET"
    objs = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            f"<< /Length {len(text)} >>\nstream\n{text}\nendstream", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    out, offsets = b"%PDF-1.4\n", []
    for i, o in enumerate(objs, 1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n{o}\nendobj\n".encode()
    xref = len(out)
    out += f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode() + b"".join(f"{o:010d} 00000 n \n".encode() for o in offsets)
    out += f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return out

def watch(page, who):
    page.on("console", lambda m: m.type == "error" and "ERR_INTERNET_DISCONNECTED" not in m.text and errors.append(f"[{who}] console: {m.text}"))
    page.on("pageerror", lambda e: errors.append(f"[{who}] pageerror: {e}"))

def shot(page, name):
    page.screenshot(path=str(OUT / f"{name}.png"), full_page=True)

def add_block(page, kind):
    btn = page.get_by_role("button", name="+ Add block")
    btn.locator("xpath=preceding-sibling::div//select").select_option(kind)
    btn.click()

def sign_in(page, user, url=BASE):
    page.goto(url)
    if "#/join/" in url:
        # An invitation link opens the sign-up page with the invited email filled
        # in; in dev mode it continues as that address's dev user ({id}@dev.local).
        expect(page.locator("input[type=email]")).to_have_value(f"{user}@dev.local")
        page.get_by_role("button", name=f"Continue as {user} (dev)").click()
    else:
        page.once("dialog", lambda d: d.accept(user))
        page.get_by_role("button", name="Sign in", exact=True).click()
    page.wait_for_load_state("networkidle")

with sync_playwright() as p:
    # A fake camera and microphone, granted without a prompt, for in-app recording.
    browser = p.chromium.launch(channel=os.environ.get("BROWSER_CHANNEL", "msedge"), headless=True,
                                args=["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])
    mobile = dict(viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True)

    # ---------------- admin invites the coach (there is no open coach signup)
    # The local API lets dev user "admin" (admin@dev.local) set up the admin
    # account; on a re-run against the same emulator it already exists.
    admin_ctx = browser.new_context(**mobile)
    admin = admin_ctx.new_page(); watch(admin, "admin")
    sign_in(admin, "admin")
    expect(admin.get_by_role("heading", name=re.compile("^(Welcome|Coaches)$")).first).to_be_visible()
    setup = admin.get_by_role("button", name="Set up admin account")
    if setup.count():
        admin.get_by_label("Your name").fill("Ada Admin")
        setup.click()
    expect(admin.get_by_role("heading", name="Coaches").first).to_be_visible()
    admin.get_by_role("button", name="+ Invite coach").click()
    admin.get_by_label("Coach name").fill("Alex Coach")
    admin.get_by_label("Email").fill(f"coach{RUN}@dev.local")
    admin.get_by_role("button", name="Send invitation").click()
    # no email locally, so the admin gets the link to share
    expect(admin.get_by_role("heading", name="Invitation link")).to_be_visible()
    coach_code = admin.locator(".sheet input").input_value().rsplit("/", 1)[-1]
    shot(admin, "00-admin-coach-invite")

    # ---------------- coach accepts the invite
    coach_ctx = browser.new_context(**mobile)
    coach = coach_ctx.new_page(); watch(coach, "coach")
    sign_in(coach, "coach" + RUN, BASE + "#/join/" + coach_code)
    expect(coach.get_by_role("heading", name="Coach on PT Coach")).to_be_visible()
    # accepting needs the data agreement ticked
    coach.get_by_role("button", name="Accept invite").click()
    expect(coach.get_by_text("Tick “I agree” to continue.")).to_be_visible()
    coach.get_by_text("How your data is used").click()
    expect(coach.get_by_text("If you list yourself in the coach directory")).to_be_visible()
    coach.get_by_label("I agree to how PT Coach uses my data").check()
    coach.get_by_role("button", name="Accept invite").click()

    # ---------------- the coach fills in their profile before anything else
    # (tasks/coaches; the API's refusals of incomplete profiles are in e2e.rs)
    expect(coach.get_by_role("heading", name="Set up your coach profile")).to_be_visible()
    for label, value in [("Legal name", "Jordan Q. Coach"), ("Phone", "512 555 0100"), ("Street address", "1 Main St"),
                         ("City", "Austin"), ("State", "TX"), ("ZIP code", "78701"), ("CPR/AED certification expires", "2030-01-01"),
                         ("Display name", f"Coach {RUN}"), ("Headline", "Strength for busy parents"),
                         ("Certification", "NSCA Certified Strength and Conditioning Specialist (CSCS)"), ("Issuer", "NSCA")]:
        coach.get_by_label(label, exact=True).fill(value)
    coach.get_by_label("About you", exact=True).fill("I help busy parents get strong in three short sessions a week, with plans that fit real life.")
    coach.get_by_label("Strength training", exact=True).check()
    coach.get_by_label("In person", exact=True).check()
    shot(coach, "00b-coach-profile")
    coach.get_by_role("button", name="Save and continue").click()
    expect(coach.get_by_role("heading", name="Clients")).to_be_visible()

    # ---------------- anyone can find the coach in the directory, without signing in
    anon_ctx = browser.new_context(**mobile)
    anon = anon_ctx.new_page(); watch(anon, "visitor")
    anon.goto(BASE)
    anon.get_by_role("link", name="Find a coach").click()
    anon.get_by_label("Search coaches").fill(f"Coach {RUN}")
    anon.locator("a.coach-card", has_text=f"Coach {RUN}").click()
    expect(anon.get_by_role("heading", name=f"Coach {RUN}")).to_be_visible()
    expect(anon.get_by_text("Jordan Q. Coach")).to_have_count(0)  # the legal name is private
    anon.get_by_role("button", name="Show email").click()
    expect(anon.get_by_role("link", name=f"Email Coach {RUN}")).to_have_attribute("href", re.compile(f"^mailto:coach{RUN}@dev.local"))
    shot(anon, "00c-coach-public-page")

    # the landing page's ask-to-join form lands in the admin's queue (the admin will deny it below)
    anon.goto(BASE)
    anon.get_by_label("Email", exact=True).fill(f"wannabe{RUN}@dev.local")
    anon.get_by_label("Coach or gym name", exact=True).fill("Coach Wanna")
    anon.wait_for_timeout(3200)  # the API drops sends faster than a person fills the form
    anon.get_by_role("button", name="Request an invitation").click()
    expect(anon.locator("#toast")).to_contain_text("club")
    anon_ctx.close()

    # ---------------- the admin sees the coach's private details
    admin.reload()
    expect(admin.get_by_role("heading", name="Coach requests")).to_be_visible()
    expect(admin.get_by_text("Reviewed by hand")).to_be_visible()
    req_row = admin.locator("div.card.spread", has_text=f"wannabe{RUN}@dev.local").first
    req_row.get_by_role("button", name="Deny").click()
    expect(admin.locator("#toast")).to_contain_text("denied")
    admin.reload()
    admin.locator(".card.link", has_text=f"coach{RUN}@dev.local").click()
    expect(admin.locator(".sheet").get_by_text("Jordan Q. Coach")).to_be_visible()
    expect(admin.locator(".sheet").get_by_text("In the directory")).to_be_visible()
    shot(admin, "00d-admin-coach-details")
    admin.locator(".sheet").get_by_role("button", name="Close").click()

    # ---------------- invite
    coach.get_by_role("button", name="+ Invite").click()
    coach.get_by_label("Client name").fill("Sam Client")
    # Invites can only be accepted by the invited address; dev users are {id}@dev.local.
    coach.get_by_label("Email").fill(f"client{RUN}@dev.local")
    coach.get_by_role("button", name="Send invitation").click()
    expect(coach.get_by_role("heading", name="Invite link")).to_be_visible()
    link = coach.locator(".sheet input").input_value()
    code = link.rsplit("/", 1)[-1]
    print("invite", code)
    shot(coach, "01-coach-invite")
    coach.get_by_role("button", name="Done").click()

    # ---------------- a would-be client asks from the coach's own sign-up link (their website's)
    asker_ctx = browser.new_context(**mobile)
    asker = asker_ctx.new_page(); watch(asker, "asker")
    asker.goto(BASE + "#/request/coach" + RUN)
    expect(asker.get_by_role("heading", name=re.compile("^Train with "))).to_be_visible()
    asker.get_by_label("Email", exact=True).fill(f"asker{RUN}@dev.local")
    asker.get_by_label("Your name", exact=True).fill("Alex Asker")
    asker.wait_for_timeout(3200)  # the API drops sends faster than a person fills the form
    shot(asker, "01b-client-request")
    asker.get_by_role("button", name="Request an invitation").click()
    expect(asker.locator("#toast")).to_contain_text("approves you")
    asker_ctx.close()
    coach.reload()
    expect(coach.get_by_text("1 waiting")).to_be_visible()
    asked = coach.locator(".request-row", has_text="Alex Asker")
    expect(asked).to_contain_text("Waiting")
    shot(coach, "01c-join-requests")
    asked.get_by_role("button", name="Invite").click()
    expect(coach.get_by_role("heading", name="Invite link")).to_be_visible()  # no email locally: share the link
    coach.get_by_role("button", name="Done").click()
    expect(coach.locator(".request-row", has_text="Alex Asker")).to_contain_text("Invited")
    coach.get_by_role("button", name="🔗 Your sign-up link").click()
    expect(coach.get_by_label("Sign-up link")).to_have_value(re.compile(f"#/request/coach{RUN}$"))
    expect(coach.get_by_label("Button HTML")).to_have_value(re.compile("^<a href="))
    coach.locator(".sheet").get_by_role("button", name="Done").click()

    # ---------------- program builder
    coach.get_by_role("link", name="Programs").click()
    coach.get_by_role("button", name="+ New").click()
    coach.get_by_label("Program name").fill("Strength Base")
    coach.get_by_label("Weeks").fill("2"); coach.get_by_label("Weeks").press("Tab")
    today_dow = datetime.date.today().weekday()  # 0 = Monday
    days = coach.locator(".cal .day")
    days.nth(today_dow).click()  # week 1, today's weekday -> lines up with a start date of the Monday this week
    add_block(coach, "straight")
    coach.locator(".block-card select").nth(1).select_option(label="Back Squat")
    expect(coach.locator(".item-card").first.get_by_label("Load unit")).to_have_value("w")  # lb, or % 1RM
    coach.locator(".item-card").first.get_by_label("Load", exact=True).fill("225")
    coach.locator(".item-card").first.get_by_label("Load", exact=True).press("Tab")
    coach.locator(".item-card").first.get_by_label("Reps", exact=True).fill("5")
    coach.locator(".item-card").first.get_by_label("Reps", exact=True).press("Tab")
    # a progression rule: suggest +5 lb next time when every set is at RPE 8 or lower
    squat_card = coach.locator(".item-card").first
    squat_card.get_by_text("Progression: None").click()
    squat_card.get_by_label("Add load (lb)").fill("5"); squat_card.get_by_label("Add load (lb)").press("Tab")
    squat_card.get_by_label("At RPE or lower").fill("8"); squat_card.get_by_label("At RPE or lower").press("Tab")
    expect(squat_card.get_by_text("Progression: +5 lb, at RPE ≤ 8")).to_be_visible()
    add_block(coach, "emom")
    coach.locator(".block-card").nth(1).locator("select").nth(1).select_option(label="Burpee")
    # a warm-up and post-workout cardio every workout gets when the program is assigned
    every = coach.locator(".card", has=coach.get_by_role("heading", name="Warm-up and cardio for every workout"))
    every.get_by_role("button", name="+ Warm-up").click()
    expect(every.locator(".block-card").nth(0).get_by_label("Block type")).to_have_value("flow")  # a warm-up is a flow unless picked
    every.locator(".block-card").nth(0).locator("select").nth(1).select_option(label="Rowing (erg)")
    every.get_by_role("button", name="+ Post-workout cardio").click()
    walk = every.locator(".block-card").nth(1)
    expect(walk.get_by_label("Block type")).to_have_value("steady")  # post-workout cardio is one steady effort
    walk.locator("select").nth(1).select_option(label="Incline Walk")
    expect(walk.get_by_label("Minutes")).to_have_value("20")  # cardio exercises start as 20 min
    walk.get_by_label("Zone", exact=True).select_option("2")
    shot(coach, "02-program-builder")
    coach.locator("button.primary", has_text="Save").first.click()
    expect(coach.get_by_text("Program saved")).to_be_visible()
    # copy week 1 to week 2
    coach.locator(".cal .wk button").first.click()
    coach.locator(".sheet").get_by_role("button", name="Copy").click()
    coach.locator("button.primary", has_text="Save").first.click()
    expect(coach.locator(".cal .day.has")).to_have_count(2)
    shot(coach, "03-program-calendar")

    # ---------------- client joins via link
    client_ctx = browser.new_context(**mobile)
    client = client_ctx.new_page(); watch(client, "client")
    sign_in(client, "client" + RUN, BASE + "#/join/" + code)
    expect(client.get_by_role("heading", name=f"Join Coach {RUN}")).to_be_visible()
    client.get_by_label("I agree to how PT Coach uses my data").check()
    client.get_by_role("button", name="Accept invite").click()
    expect(client.get_by_role("heading", name=re.compile("Step 1 of 2"))).to_be_visible()
    expect(client.locator("#title")).to_have_text(f"Coach {RUN}")  # the header shows the client's coach
    client.get_by_label("Goals").fill("Squat 140 kg by summer")
    client.get_by_label("Sex (for body-fat equations)").select_option("male")
    client.get_by_label("Birth date").fill("1994-03-02")
    client.get_by_label("Height (in)").fill("71")
    client.get_by_role("button", name="Next").click()
    expect(client.get_by_role("heading", name=re.compile("Step 2 of 2"))).to_be_visible()
    for r in client.get_by_label("No", exact=True).all():
        r.check()
    client.get_by_label("Signature").fill("Sam Client")
    client.get_by_label("I have read and agree to the waiver").check()
    shot(client, "04-waiver")
    client.get_by_role("button", name="Sign", exact=True).click()
    expect(client.get_by_role("heading", name="Today")).to_be_visible()

    # ---------------- coach assigns the program starting this Monday
    monday = datetime.date.today() - datetime.timedelta(days=today_dow)
    coach.get_by_role("link", name="Clients").click()
    coach.get_by_text("Sam Client").first.click()
    coach.get_by_role("button", name="Calendar").click()
    coach.get_by_label("Start date").fill(monday.isoformat())
    expect(coach.get_by_label("Program")).to_have_value(re.compile(r"^p:"))  # the coach's template comes first
    coach.get_by_role("button", name="Assign").click()
    expect(coach.get_by_text("has their own copy to personalize")).to_be_visible()
    # the assignment ran from Sam's own copy, listed with an Edit link
    expect(coach.get_by_role("heading", name="Sam Client’s programs")).to_be_visible()
    expect(coach.locator(".card", has_text="Sam Client’s programs").get_by_role("link", name="Edit")).to_be_visible()
    shot(coach, "05-coach-calendar")

    # ---------------- client logs today's workout, offline
    client.reload(); client.wait_for_load_state("networkidle")
    client.locator(".card.today").first.click()
    expect(client.get_by_role("heading", name=re.compile("Week 1"))).to_be_visible()
    # the ⓘ beside RPE explains how to rate a set
    client.get_by_role("button", name="What is RPE?").first.click()
    expect(client.locator(".sheet").get_by_text("Could have done 2 more reps")).to_be_visible()
    shot(client, "06b-rpe-info")
    client.locator(".sheet").get_by_role("button", name="Got it").click()
    client_ctx.set_offline(True)
    # the program's warm-up comes first and is ticked off as a whole
    expect(client.locator(".card.phase-warmup")).to_contain_text("Rowing (erg)")
    expect(client.locator(".card.phase-warmup h3")).to_contain_text("Warm-up · Flow · once through")
    client.get_by_role("button", name="What is Flow?").click()  # each type explains itself to the client
    expect(client.locator(".sheet")).to_contain_text("Go through each move once, in order")
    client.locator(".sheet").get_by_role("button", name="Got it").click()
    client.get_by_role("button", name="Mark warm-up done").click()
    expect(client.get_by_role("button", name="✓ Warm-up done")).to_be_visible()
    done = client.locator(".setrow .done")
    print("set rows", done.count())
    done.nth(0).click()
    client.get_by_role("button", name="Skip", exact=True).click()  # dismiss rest timer
    done.nth(1).click(); client.get_by_role("button", name="Skip", exact=True).click()
    client.locator(".setrow").nth(2).locator("input").first.fill("4")
    client.locator(".setrow").nth(2).locator("input").first.press("Tab")
    done.nth(2).click(); client.get_by_role("button", name="Skip", exact=True).click()
    client.locator(".card", has=client.get_by_role("heading", name=re.compile("^EMOM"))).get_by_role("button", name="⏱ Timer").click()
    time.sleep(1.2)
    shot(client, "06-emom-timer")
    client.locator(".sheet").get_by_role("button", name="Close").click()
    shot(client, "07-logger")
    # the cardio after the workout: prefilled with the prescription, plus a heart rate
    walk_card = client.locator(".card.phase-post")
    expect(walk_card).to_contain_text("Zone 2 (")  # shown in bpm from the client's age
    expect(walk_card.get_by_label("minutes")).to_have_value("20")
    walk_card.get_by_label("average heart rate").fill("128")
    walk_card.get_by_label("average heart rate").press("Tab")
    walk_card.locator(".setrow .done").click()
    shot(client, "07a-cardio")
    # a note on the squats for the coach, and a rating for the whole workout
    client.get_by_label("Note on Back Squat").fill("Left hip a bit tight on the way down")
    client.get_by_role("radio", name="4 Good").click()
    expect(client.get_by_role("radio", name="4 Good")).to_have_attribute("aria-checked", "true")
    shot(client, "07b-note-and-rating")
    client.get_by_role("button", name="Finish workout").click()
    client.get_by_label("Comment").fill("Last set was a grind")
    client.get_by_label("Session RPE (1-10)").fill("8")
    client.get_by_label("I felt pain").check()
    client.get_by_placeholder("Where? e.g. left knee").fill("left knee")
    client.locator(".sheet").get_by_role("button", name="Save").click()
    expect(client.get_by_text(re.compile("Saved offline"))).to_be_visible()
    expect(client.locator("#sync")).to_contain_text("Offline")
    shot(client, "08-offline-today")
    client_ctx.set_offline(False)
    client.evaluate("window.dispatchEvent(new Event('online'))")
    expect(client.locator("#sync")).to_be_hidden(timeout=10000)
    print("synced after reconnect")

    # ---------------- client attaches a video to a set (online; goes straight to S3)
    client.locator(".card.today").first.click()
    client.locator(".videobar").first.get_by_role("button", name="🎥 Add video").click()
    client.locator(".sheet input[type=file]").set_input_files(str(Path(__file__).parent / "fixtures" / "clip.mp4"))
    # trim (VS-2): a 65 s clip starts with its first 60 s selected; pulling the
    # end handle to 1:05 pushes the start to 0:05 to stay within 60 s
    readout = client.locator(".trim span[aria-live]")
    expect(readout).to_contain_text("0:00.0 – 1:00.0 · 60.0 s")
    client.locator(".trim input[aria-label=End]").evaluate(
        "(el) => { el.value = '65'; el.dispatchEvent(new Event('input', { bubbles: true })); }")
    expect(readout).to_contain_text("0:05.0 – 1:05.0 · 60.0 s")
    shot(client, "08a-trim")
    # Browsers with an H.264 encoder convert the selection on the device; the
    # others (e.g. Playwright's bundled Chromium) upload the original with trim points.
    converts = client.evaluate("""async () => 'VideoEncoder' in window &&
        (await VideoEncoder.isConfigSupported({ codec: 'avc1.42e01f', width: 160, height: 120, bitrate: 2500000 })).supported""")
    print("video path:", "convert on device" if converts else "upload original with trim points")
    client.locator(".sheet").get_by_role("button", name="Upload").click()
    expect(client.locator(".videobar").first.get_by_role("button", name=re.compile("▶ Set"))).to_be_visible(timeout=30000)
    shot(client, "08b-set-video")
    # the fallback for devices that can't convert: the original uploads with trim points
    client.evaluate("localStorage.setItem('pt.forceVideoFallback', '1')")
    client.locator(".videobar").nth(1).get_by_role("button", name="🎥 Add video").click()
    client.locator(".sheet input[type=file]").set_input_files(str(Path(__file__).parent / "fixtures" / "clip.mp4"))
    expect(readout).to_contain_text("60.0 s")
    client.locator(".trim input[aria-label=End]").evaluate(
        "(el) => { el.value = '30'; el.dispatchEvent(new Event('input', { bubbles: true })); }")
    expect(readout).to_contain_text("0:00.0 – 0:30.0 · 30.0 s")
    client.locator(".sheet").get_by_role("button", name="Upload").click()
    expect(client.locator(".videobar").nth(1).get_by_role("button", name=re.compile("▶ Set"))).to_be_visible(timeout=30000)
    client.evaluate("localStorage.removeItem('pt.forceVideoFallback')")
    # in-app recording (VS-1): about 3 s from the fake camera, as set 1 of the squats
    client.locator(".videobar").first.get_by_role("button", name="🎥 Add video").click()
    # filming guidance for the squat (VS-5) before the camera opens
    expect(client.locator(".film-tip")).to_contain_text("From 45°, between side and front, about 10 ft away")
    client.locator(".sheet select").first.select_option("0")
    client.get_by_role("button", name="● Record with camera").click()
    rec = client.get_by_role("button", name="● Record", exact=True)
    expect(rec).to_be_enabled(timeout=10000)
    shot(client, "08c-recorder")
    rec.click()
    client.wait_for_timeout(3000)
    client.get_by_role("button", name="■ Stop").click()
    expect(readout).to_contain_text(re.compile(r"0:00\.0 – 0:0[2-4]"))
    client.locator(".sheet").get_by_role("button", name="Upload").click()
    expect(client.locator(".videobar").first.get_by_role("button", name="▶ Set 1")).to_be_visible(timeout=30000)

    # offline (VS-3): a clip saved with no connection waits on the device...
    fixture = str(Path(__file__).parent / "fixtures" / "clip.mp4")
    burpee_bar = client.locator(".videobar").nth(1)
    client_ctx.set_offline(True)
    client.evaluate("window.dispatchEvent(new Event('offline'))")
    burpee_bar.get_by_role("button", name="🎥 Add video").click()
    client.locator(".sheet select").first.select_option("1")
    client.locator(".sheet input[type=file]").set_input_files(fixture)
    expect(readout).to_contain_text("60.0 s")
    client.locator(".trim input[aria-label=End]").evaluate(
        "(el) => { el.value = '10'; el.dispatchEvent(new Event('input', { bubbles: true })); }")
    client.locator(".sheet").get_by_role("button", name="Upload").click()
    expect(burpee_bar.get_by_role("button", name="⏳ Set 2 · waiting for connection")).to_be_visible(timeout=30000)
    shot(client, "08d-queued-offline")
    # ...starts by itself when the connection returns, and when the part upload
    # is cut off, a reload carries on from where it stopped with fresh part URLs
    cut = []
    def cut_part(route):
        cut.append(route.request.url)
        route.abort("internetdisconnected")  # like losing the connection mid-upload
    client.route(re.compile(r".*partNumber=\d+.*"), cut_part)
    client_ctx.set_offline(False)
    client.evaluate("window.dispatchEvent(new Event('online'))")
    for _ in range(100):
        if cut: break
        client.wait_for_timeout(100)
    assert cut, "the queued upload didn't start when the connection came back"
    client.unroute(re.compile(r".*partNumber=\d+.*"))
    client.reload()
    expect(client.locator(".videobar").nth(1).get_by_role("button", name="▶ Set 2")).to_be_visible(timeout=30000)
    # the client can delete their own video
    client.locator(".videobar").nth(1).get_by_role("button", name="▶ Set 2").click()
    client.locator(".sheet").get_by_role("button", name="Delete video").click()
    client.locator(".sheet").last.get_by_role("button", name="Delete", exact=True).click()
    expect(client.locator(".videobar").nth(1).get_by_role("button", name="▶ Set 2")).to_have_count(0)
    expect(client.get_by_text("Video deleted")).to_be_visible()
    client.get_by_role("button", name="‹ Back").click()

    # ---------------- client body metrics
    client.get_by_role("link", name="Progress").click()
    for d, w in [(-14, 185.0), (-7, 183.4), (0, 182.0)]:
        client.get_by_label("Date").fill((datetime.date.today() + datetime.timedelta(days=d)).isoformat())
        client.get_by_label("Weight (lb)").fill(str(w))
        client.get_by_role("button", name="Log weight").click()
        expect(client.get_by_text("Saved").first).to_be_visible()
        client.wait_for_timeout(300)
    shot(client, "09-weight")
    # the starting point (ON-4): first entries by default; set to a week ago, the change is from 183.4
    expect(client.get_by_text("📍 Starting point: first entries")).to_be_visible()
    expect(client.get_by_text(re.compile(r"Since the starting point \(185 lb"))).to_be_visible()
    client.locator(".start-point").get_by_role("button", name="Change").click()
    client.locator(".sheet").get_by_label("Date").fill((datetime.date.today() - datetime.timedelta(days=7)).isoformat())
    client.locator(".sheet").get_by_role("button", name="Save", exact=True).click()
    expect(client.get_by_text("Starting point saved")).to_be_visible()
    expect(client.get_by_text(re.compile(r"Since the starting point \(183\.4 lb"))).to_be_visible()
    expect(client.locator(".card", has_text="Since the starting point")).to_contain_text("-1.4 lb")
    client.get_by_role("button", name="Body fat").click()
    inputs = client.locator(".sheet, .card").first.locator("input[placeholder='mm']")
    for i, v in enumerate([15, 25, 20]):
        inputs.nth(i).fill(str(v))
    client.get_by_role("button", name="Calculate & save").click()
    expect(client.get_by_text("Body fat").nth(1)).to_be_visible()
    shot(client, "10-skinfold")

    # ---------------- 1RMs: the logged squat offers an estimate; record a tested max
    client.get_by_role("button", name="Maxes").click()
    expect(client.get_by_role("heading", name="From logged sets")).to_be_visible()
    client.get_by_label("Exercise").select_option(label="Back Squat")
    client.get_by_label("1RM (lb)").fill("275")
    client.get_by_role("button", name="Save max").click()
    expect(client.locator(".max-row", has_text="Back Squat").get_by_text("275 lb")).to_be_visible()
    # Edit fills the form with the max to change it
    client.get_by_role("button", name="Edit Back Squat max").click()
    expect(client.get_by_label("1RM (lb)")).to_have_value("275")
    client.get_by_label("1RM (lb)").fill("280")
    client.get_by_role("button", name="Save max").click()
    expect(client.locator(".max-row", has_text="Back Squat").get_by_text("280 lb")).to_be_visible()
    shot(client, "10b-maxes")
    # the logged squat set the client's starting records
    client.get_by_role("button", name="Records").click()
    expect(client.locator(".card", has_text="Back Squat").get_by_text("5-rep max: 225 lb")).to_be_visible()

    # ---------------- weekly check-in (FB-3): from Today, answered, seen by the coach
    client.get_by_role("link", name="Today").click()
    client.get_by_role("link", name="Start check-in").click()
    expect(client.get_by_role("heading", name="Weekly check-in")).to_be_visible()
    client.get_by_role("button", name="Send check-in").click()
    expect(client.get_by_text("“How was your week overall?” needs an answer")).to_be_visible()
    for label, n in [("How was your week overall?", "8"), ("Energy", "7"), ("Sleep quality", "6"), ("Stress (10 = very stressed)", "4")]:
        client.get_by_role("radiogroup", name=label).get_by_role("radio", name=n, exact=True).click()
    client.get_by_label("Wins this week").fill("Squat felt fast")
    client.get_by_role("button", name="Send check-in").click()
    expect(client.get_by_text("Weekly check-in sent.")).to_be_visible()
    coach.goto(coach.url.split("#")[0] + "#/c/client" + RUN + "/checkins")
    expect(coach.locator("table.checkins")).to_contain_text("Squat felt fast")
    shot(coach, "10c-coach-checkins")

    # ---------------- daily habits (BZ-3): added and tapped on Today, seen by the coach
    client.get_by_role("link", name="Today").click()
    client.get_by_role("button", name="+ Add a daily habit").click()
    client.get_by_label("Habit", exact=True).fill("Water")
    client.get_by_label("Kind").select_option("count")
    client.get_by_label("Daily target").fill("8")
    client.get_by_label("Unit").fill("glasses")
    client.get_by_role("button", name="Add habit").click()
    # adding closes the sheet once it's saved (and Today redraws with the tile)
    expect(client.get_by_text("Habit added")).to_be_visible()
    expect(client.locator(".sheet")).to_have_count(0)
    water = client.locator(".habit", has_text="Water")
    for _ in range(3):
        water.get_by_role("button", name="One more Water").click()
    expect(water).to_contain_text("3 / 8 glasses")
    water.get_by_label("Water today").fill("8")
    water.get_by_label("Water today").press("Tab")
    expect(water).to_contain_text("🔥 1")
    expect(water).to_have_class(re.compile("done"))
    shot(client, "10d-habits")
    expect(client.locator("#sync")).to_be_hidden(timeout=15000)  # the queued values have synced
    # The coach's card shows the server's streak as it loads, so reload until
    # the last value (8, typed straight after the taps) has arrived. A value
    # lost on the way (the outbox once deleted an edit queued mid-send) never
    # arrives, and this still fails.
    coach.goto(coach.url.split("#")[0] + "#/c/client" + RUN + "/overview")
    row = coach.locator(".habit-row", has_text="Water")
    for _ in range(15):
        expect(row).to_be_visible()
        if "🔥 1" in row.inner_text():
            break
        time.sleep(1)
        coach.reload()
    expect(row).to_contain_text("🔥 1")
    expect(coach.get_by_role("heading", name="Weekly cardio (minutes)")).to_be_visible()  # the 20 min walk, not the warm-up
    # the printable progress report (BZ-6), from the client's Overview
    coach.get_by_role("link", name="📄 Progress report (print or PDF)").click()
    expect(coach.get_by_role("heading", name="Progress report")).to_be_visible()
    expect(coach.locator(".report")).to_contain_text("Strength: estimated 1RM")
    expect(coach.locator(".report")).to_contain_text("Weekly check-ins")
    expect(coach.locator(".report")).to_contain_text("20 minutes in total")
    expect(coach.locator(".report")).to_contain_text("1 of 1 (100%)")  # the habit counts from the day it was added
    coach.get_by_label("Notes for the report").fill("Great first weeks. Next: push the squat to 245.")
    coach.emulate_media(media="print")
    expect(coach.get_by_role("button", name="Print / Save as PDF")).to_be_hidden()
    shot(coach, "10e-report-print")
    coach.emulate_media(media="screen")
    coach.get_by_role("link", name="‹ Back").click()

    # ---------------- nutrition (BZ-4): a meal, an imported diary, targets, an estimate by hand
    client.get_by_role("link", name="Today").click()
    client.get_by_role("button", name="+ Log a meal").click()
    client.get_by_label("One item per line").fill("chicken 12oz\nrice")
    client.get_by_role("button", name="Save meal").click()
    expect(client.locator(".meal-line").first).to_contain_text("not estimated yet")
    yesterday = (datetime.date.today() - datetime.timedelta(days=1)).isoformat()
    client.get_by_role("button", name="Import from Cronometer or MyFitnessPal").click()
    client.locator(".sheet input[type=file]").set_input_files(files=[{
        "name": "servings.csv", "mimeType": "text/csv",
        "buffer": f"Day,Time,Group,Food Name,Amount,Energy (kcal),Carbs (g),Fat (g),Protein (g)\n{yesterday},8:00 AM,Breakfast,\"Oats, rolled\",40.00 g,150,27,2.5,5\n".encode(),
    }])
    client.locator(".sheet").get_by_role("button", name="Import", exact=True).click()
    expect(client.locator(".sheet")).to_contain_text("Imported 1 meal over 1 day")
    client.locator(".sheet").get_by_role("button", name="Close").click()
    expect(client.locator(".meal-line", has_text="Breakfast")).to_contain_text("150 kcal")
    expect(client.locator("#sync")).to_be_hidden(timeout=15000)
    coach.reload()
    coach.get_by_role("button", name="Set targets").click()
    coach.get_by_label("Calories (kcal)").fill("2200")
    coach.get_by_label("Protein (g)").fill("160")
    coach.get_by_role("button", name="Save", exact=True).click()
    expect(coach.get_by_text("Targets: 2200 kcal · Protein 160 g")).to_be_visible()
    coach.get_by_role("button", name="Estimate", exact=True).click()
    coach.get_by_label("Calories for chicken").fill("560")
    coach.get_by_label("Calories for rice").fill("200")
    coach.locator(".sheet").get_by_role("button", name="Save", exact=True).click()
    expect(coach.get_by_text("Meal saved")).to_be_visible()
    shot(coach, "10f-nutrition-coach")
    client.reload()
    expect(client.locator(".meter-row", has_text="Calories")).to_contain_text("760 / 2,200 kcal")
    expect(client.locator(".meter-row", has_text="Protein")).to_contain_text("0 / 160 g")
    shot(client, "10f-nutrition-today")

    # ---------------- body fat from tape (BM-6): the US Navy estimate
    client.get_by_role("link", name="Progress").click()
    client.get_by_role("button", name="Body fat").click()
    client.get_by_role("button", name="Tape (US Navy)").click()
    client.get_by_label("Neck (in)").fill("15")
    client.get_by_label("Waist (in)").fill("33.5")
    if not client.get_by_label("Height (in)").input_value():
        client.get_by_label("Height (in)").fill("71")
    client.get_by_role("button", name="Calculate & save").click()
    expect(client.get_by_text(re.compile(r"Saved: \d+(\.\d)?% body fat"))).to_be_visible()
    expect(client.locator("table").last).to_contain_text("Tape (US Navy)")

    # ---------------- the coach's own intake questions (ON-5)
    coach.get_by_role("link", name="Profile").click()
    expect(coach.get_by_role("heading", name="Your coaching time")).to_be_visible()
    coach.get_by_role("button", name="Edit intake questions").click()
    coach.get_by_role("button", name="+ Add a question").click()
    coach.locator(".sheet").get_by_label("Question").fill("Your main sport")
    coach.locator(".sheet").get_by_label("Answer").select_option("text")
    coach.locator(".sheet").get_by_label("Required").check()
    coach.locator(".sheet").get_by_role("button", name="Save", exact=True).click()
    expect(coach.get_by_text("Intake questions saved")).to_be_visible()
    client.get_by_role("link", name="Profile").click()
    client.get_by_role("link", name="Intake questionnaire").click()
    expect(client.get_by_role("heading", name="From your coach")).to_be_visible()
    client.get_by_label("Your main sport").fill("Powerlifting")
    client.get_by_role("button", name="Save", exact=True).click()
    expect(client.locator("#toast")).to_contain_text("Saved")
    coach.goto(coach.url.split("#")[0] + "#/c/client" + RUN + "/intake")
    expect(coach.get_by_text("Powerlifting")).to_be_visible()

    # ---------------- coach dashboard + feedback
    coach.get_by_role("button", name="Overview").click()
    print("coach url", coach.url)
    expect(coach.get_by_text("Weekly volume (lb × reps)")).to_be_visible()
    shot(coach, "11-coach-overview")
    coach.get_by_role("button", name="Logs").click()
    expect(coach.get_by_text("Left hip a bit tight on the way down")).to_be_visible()
    expect(coach.get_by_text("Rated 4/5 · Good")).to_be_visible()
    coach.locator(".exercise", has_text="Back Squat").locator(".row button").nth(2).click()  # set 3 of the squats
    expect(coach.get_by_role("heading", name="Comment on set 3")).to_be_visible()
    # save a snippet (it starts from what's typed), then insert it with the / shortcut
    comment = coach.get_by_label("Comment", exact=True)
    comment.fill("Great depth, {name}. Keep the chest up on set 3")
    coach.get_by_role("button", name="Insert a saved snippet").click()
    coach.get_by_role("button", name="+ New snippet").click()
    coach.get_by_label("Name", exact=True).fill("depth")
    coach.get_by_role("button", name="Save", exact=True).click()
    expect(coach.get_by_text("Snippet saved")).to_be_visible()
    coach.locator(".sheet").last.get_by_role("button", name="Close").click()
    comment.fill("")
    comment.press_sequentially("/dep")
    expect(coach.locator(".snippet-opt", has_text="depth")).to_be_visible()
    comment.press("Enter")
    expect(comment).to_have_value(re.compile(r"^Great depth, \w+\. Keep the chest up"))
    coach.locator(".sheet").get_by_role("button", name="Send").click()
    expect(coach.get_by_text(re.compile("Keep the chest up"))).to_be_visible()
    shot(coach, "12-coach-logs")
    # the client's set video plays for the coach
    squat = coach.locator(".exercise", has_text="Back Squat")
    squat.locator(".videobar button", has_text="▶ Set 3").click()
    expect(coach.locator(".sheet video.player")).to_be_visible()
    coach.wait_for_function("document.querySelector('.sheet video.player').readyState >= 1", timeout=15000)
    if converts:
        # converted: just the 60 s selection, with a poster frame
        coach.wait_for_function("Math.abs(document.querySelector('.sheet video.player').duration - 60) < 1", timeout=10000)
        assert coach.locator(".sheet video.player").get_attribute("poster"), "converted video has no poster"
        expect(coach.locator(".sheet")).not_to_contain_text("of the original")
    else:
        # original: the player keeps to the trim points, starting at 0:05
        coach.wait_for_function("Math.abs(document.querySelector('.sheet video.player').currentTime - 5) < 0.5", timeout=10000)
        expect(coach.locator(".sheet")).to_contain_text("Plays 5.0–65.0 s of the original")
    shot(coach, "12b-coach-video")
    coach.locator(".sheet").get_by_role("button", name="Close").click()
    # the burpees' video took the fallback: the original, kept to 0–30 s
    coach.locator(".exercise", has_text="Burpee").locator(".videobar button", has_text="▶ Set").first.click()
    expect(coach.locator(".sheet")).to_contain_text("Plays 0.0–30.0 s of the original")
    coach.wait_for_function("document.querySelector('.sheet video.player').readyState >= 1", timeout=15000)
    coach.locator(".sheet").get_by_role("button", name="Close").click()
    # the in-app recording plays too
    squat.locator(".videobar button", has_text="▶ Set 1").click()
    coach.wait_for_function("document.querySelector('.sheet video.player').readyState >= 1", timeout=15000)
    coach.locator(".sheet").get_by_role("button", name="Close").click()

    # ---------------- coach review queue (VS-4): oldest first, mark reviewed
    coach.locator('#tabs a[data-path="/videos"]').click()
    expect(coach.get_by_role("heading", name="Videos")).to_be_visible()
    items = coach.locator(".queue-item")
    expect(items).to_have_count(3)
    badge = coach.locator('#tabs a[data-path="/videos"] .badge')
    expect(badge).to_have_text("3")
    expect(items.nth(0)).to_contain_text("Back Squat · set 3")  # uploaded first
    expect(items.nth(1)).to_contain_text("Burpee · set 1")
    expect(items.nth(2)).to_contain_text("Back Squat · set 1")
    shot(coach, "12c-video-queue")
    items.nth(0).get_by_role("button", name="✓ Reviewed").click()
    expect(items).to_have_count(2)
    expect(badge).to_have_text("2")
    # from the player too
    items.nth(0).get_by_role("button", name="▶ Watch").click()
    coach.locator(".sheet").get_by_role("button", name="✓ Mark reviewed").click()
    expect(items).to_have_count(1)
    expect(items.nth(0)).to_contain_text("Back Squat · set 1")
    # reviewed videos are marked in the client's logs
    coach.go_back()
    expect(coach.locator(".exercise", has_text="Back Squat").locator(".videobar button", has_text="▶ Set 3 ✓")).to_be_visible()
    coach.get_by_role("link", name="Clients").click()
    expect(coach.get_by_text("Pain reported")).to_be_visible()
    # the alert inbox (LG-7): the reported pain links to the workout, then is marked done
    alert = coach.locator(".alert-row.pain").first
    expect(alert).to_contain_text("Reported pain")
    shot(coach, "13-roster")
    alert.get_by_role("link", name="See workout").click()
    expect(coach).to_have_url(re.compile(r"#/c/[^/]+/logs/"))
    expect(coach.locator(".card.focus")).to_be_visible()
    coach.get_by_role("link", name="Clients").click()
    coach.locator(".alert-row.pain").first.get_by_role("button", name="✓ Done").click()
    expect(coach.locator(".alert-row.pain")).to_have_count(0)

    client.locator('#tabs a[data-path="/coach"]').click()
    expect(client.get_by_text(re.compile("Keep the chest up"))).to_be_visible()
    shot(client, "14-client-messages")
    # the coach's set comment links to that workout (MS-3)
    about = client.locator("a.about", has_text="Back Squat set 3").first
    expect(about).to_be_visible()
    about.click()
    expect(client).to_have_url(re.compile(r"#/workout/"))
    expect(client.get_by_role("button", name="💬 Ask your coach about this workout")).to_be_visible()
    client.locator('#tabs a[data-path="/coach"]').click()

    # ---------------- the coach replies to a set video, voice over the clip (VR-1..VR-3)
    coach.locator('#tabs a[data-path="/videos"]').click()
    coach.locator(".queue-item").first.get_by_role("button", name="🎙 Reply").click()
    expect(coach.locator("canvas.review-canvas")).to_be_visible()
    coach.wait_for_function("document.querySelector('video.review-src').readyState >= 2", timeout=15000)
    coach.get_by_role("button", name="0.25× speed").click()
    coach.get_by_role("button", name="In", exact=True).click()
    for _ in range(6):
        coach.get_by_role("button", name="Forward one frame").click()
    before = coach.evaluate("document.querySelector('video.review-src').currentTime")
    assert 0.15 < before < 0.25, before  # six frames at 1/30 s
    coach.get_by_role("button", name="Out", exact=True).click()
    expect(coach.get_by_text(re.compile(r"^1\. 0:00\.0 – 0:00\.2"))).to_be_visible()
    # drawing over the clip (VR-4): an angle (three taps), an arrow, the timer; drawn into the recording
    assert coach.evaluate("import('./js/views/draw.js').then((m) => Math.round(m.angleAt({x:0,y:0},{x:0,y:10},{x:10,y:10})))") == 90
    canvas = coach.locator("canvas.review-canvas")
    box = canvas.bounding_box()
    blank = coach.evaluate("document.querySelector('canvas.review-canvas').toDataURL()")
    coach.get_by_role("button", name="Angle (tap three points)").click()
    for fx, fy in [(0.3, 0.3), (0.3, 0.7), (0.7, 0.7)]:
        coach.mouse.click(box["x"] + box["width"] * fx, box["y"] + box["height"] * fy)
    coach.get_by_role("button", name="Arrow").click()
    coach.mouse.move(box["x"] + box["width"] * 0.6, box["y"] + box["height"] * 0.2)
    coach.mouse.down(); coach.mouse.move(box["x"] + box["width"] * 0.8, box["y"] + box["height"] * 0.4, steps=5); coach.mouse.up()
    coach.get_by_role("button", name="Timer").click()
    coach.wait_for_timeout(300)
    assert coach.evaluate("document.querySelector('canvas.review-canvas').toDataURL()") != blank, "the marks are on the canvas"
    shot(coach, "17-review-drawing")
    coach.get_by_role("button", name="Undo the last mark").click()
    coach.keyboard.press("Escape")  # back to no tool, so the canvas scrolls again
    expect(coach.get_by_role("button", name="No tool (scroll and tap)")).to_have_attribute("aria-pressed", "true")
    # compare (VR-5): the client's other squat clip, side by side then overlaid, kept in step
    narrow = coach.evaluate("document.querySelector('canvas.review-canvas').width")
    compare = coach.get_by_label("Compare with")
    compare.select_option(index=1)
    coach.wait_for_function(f"document.querySelector('canvas.review-canvas').width > {narrow}", timeout=15000)
    coach.get_by_role("button", name="Forward one frame").click()
    coach.wait_for_timeout(300)
    drift = coach.evaluate("""() => { const [a, b] = document.querySelectorAll('video.review-src'); return Math.abs(a.currentTime - b.currentTime); }""")
    assert drift < 0.1, drift  # both clips start at 0 here
    shot(coach, "18-review-compare")
    coach.get_by_role("button", name="Overlay").click()
    coach.wait_for_function(f"document.querySelector('canvas.review-canvas').width === {narrow}")
    compare.select_option("")
    # the coach's camera in a corner of the reply (VR-6); the fake camera stands in
    coach.get_by_label("Show my camera").check()
    expect(coach.get_by_label("Camera corner")).to_be_enabled()
    coach.wait_for_function("[...document.querySelectorAll('video.review-src')].some((v) => v.srcObject && v.videoWidth > 0)", timeout=15000)
    coach.get_by_label("Camera corner").select_option("tl")
    coach.get_by_role("button", name="● Record reply").click()
    coach.get_by_role("button", name="Play", exact=True).click()
    time.sleep(2.5)
    coach.get_by_role("button", name="■ Stop recording").click()
    expect(coach.get_by_role("heading", name=re.compile("Your reply"))).to_be_visible()
    coach.get_by_label("Note", exact=True).fill("Knees out at the bottom")
    shot(coach, "13b-review-reply")
    coach.get_by_role("button", name="Send reply").click()
    # sent in the background; once it lands the video is reviewed and the queue empties
    expect(coach.locator('#tabs a[data-path="/videos"] .badge')).to_have_count(0, timeout=30000)
    # the client gets it in their messages without a reload, and plays it
    reply_btn = client.get_by_role("button", name="▶ Play video reply")
    expect(reply_btn).to_be_visible(timeout=20000)
    expect(client.get_by_text("Knees out at the bottom")).to_be_visible()
    reply_btn.click()
    client.wait_for_function("document.querySelector('.sheet video.player').readyState >= 1", timeout=15000)
    shot(client, "14b-client-reply")
    client.locator(".sheet").get_by_role("button", name="Close").click()

    # ---------------- messages: the coach's inbox updates live; inline reply
    coach.get_by_role("link", name="Messages").click()
    expect(coach.get_by_role("heading", name="Messages")).to_be_visible()
    client.get_by_label("Message", exact=True).fill("Can we swap Friday for Saturday?")
    client.get_by_role("button", name="Send", exact=True).click()
    # no reload: the page refreshes itself (by polling locally; live events on AWS)
    expect(coach.get_by_text("Can we swap Friday for Saturday?")).to_be_visible(timeout=20000)
    expect(coach.locator('#tabs a[data-path="/messages"] .badge')).to_be_visible()
    coach.get_by_label("Only new messages").check()
    shot(coach, "15-coach-messages")
    coach.get_by_role("button", name="Reply").first.click()
    coach.get_by_label("Reply to Sam Client").fill("Saturday works, see you then")
    coach.get_by_role("button", name="Send", exact=True).click()
    expect(coach.get_by_text("No new messages.")).to_be_visible()
    expect(coach.locator('#tabs a[data-path="/messages"] .badge')).to_have_count(0)
    expect(client.get_by_text("Saturday works, see you then")).to_be_visible(timeout=20000)
    coach.get_by_label("Only new messages").uncheck()

    # ---------------- a photo and a voice note in the conversation
    png = tiny_png()
    client.get_by_label("Photo to send").set_input_files({"name": "form.png", "mimeType": "image/png", "buffer": png})
    expect(client.locator("img.msg-photo").last).to_be_visible(timeout=15000)
    client.get_by_role("button", name="Record a voice note").click()
    time.sleep(1.5)
    client.get_by_role("button", name="Record a voice note").click()  # stop
    client.get_by_role("button", name="Send voice note").click()
    expect(client.locator("audio.msg-voice").last).to_be_visible(timeout=15000)
    shot(client, "14c-photo-and-voice")
    # the coach's inbox says what came in, without a reload
    expect(coach.get_by_text("🎤 Voice note")).to_be_visible(timeout=20000)

    # ---------------- notifications: offered in the thread; the switch on Profile
    expect(client.get_by_text("Get a notification when your coach replies?")).to_be_visible()
    client_ctx.grant_permissions(["notifications"])
    client.get_by_role("link", name="Profile").click()
    client.get_by_label("Notify me about new messages and video replies").check()
    # no VAPID key on the local server, so it says so rather than failing
    expect(client.get_by_text("Notifications aren’t set up on this server yet.")).to_be_visible()

    # ---------------- scheduling (BZ-2): the coach's hours, a client booking, the calendar feed
    coach.get_by_role("link", name="Clients").click()
    coach.get_by_role("link", name="📅 Schedule").click()
    expect(coach.get_by_role("heading", name="When clients can book")).to_be_visible()
    for day in range(7):
        coach.get_by_role("button", name="+ Add hours").click()
        coach.locator(".avail-row").nth(day).get_by_label("Day").select_option(str(day))
    coach.get_by_label("Booking notice (hours)").fill("0")
    coach.get_by_label("Location").fill("Iron Gym")
    coach.get_by_role("button", name="Save hours").click()
    expect(coach.get_by_text("Saved")).to_be_visible()
    client.get_by_role("link", name="Today").click()
    client.get_by_role("link", name=re.compile("Book a session with your coach")).click()
    expect(client.get_by_role("heading", name="Book a session")).to_be_visible()
    client.locator(".slot-grid button").first.click()
    client.get_by_role("button", name="Book", exact=True).click()
    expect(client.get_by_text("Booked; your coach has been told")).to_be_visible()
    expect(client.locator(".session-row")).to_have_count(1)
    shot(client, "19-client-sessions")
    coach.reload()
    expect(coach.locator(".session-row", has_text="Sam Client")).to_be_visible()
    client.get_by_role("link", name="Profile").click()
    expect(client.get_by_label("Calendar feed link")).to_have_value(re.compile(r"/cal/[a-z0-9]+\.ics$"))
    # the coach's hours on the client's Messages screen (MS-5)
    client.locator('#tabs a[data-path="/coach"]').click()
    expect(client.locator(".coach-hours")).to_contain_text("Usually available: Mon, Tue, Wed, Thu, Fri, Sat, Sun 8 am–12 pm")

    # ---------------- reminders and automated messages (MS-4)
    coach.get_by_role("link", name="Profile").click()
    coach.get_by_role("link", name="Set up reminders & messages").click()
    expect(coach.get_by_role("heading", name="Reminders & automated messages")).to_be_visible()
    expect(coach.get_by_label("Weekly check-in reminder time")).to_have_value("17")  # on by default, Sunday 5 pm
    coach.get_by_role("button", name="+ Add an automated message").click()
    coach.get_by_label("Message", exact=True).fill("New week, {name}! Check your program for this week.")
    coach.get_by_role("button", name="Save", exact=True).click()
    expect(coach.get_by_text("Saved")).to_be_visible()
    shot(coach, "16-automation")

    # ---------------- starter programs: the coach copies the strength benchmark into their programs
    coach.get_by_role("link", name="Programs").click()
    bench = coach.locator(".card.starter", has_text="Strength benchmark (1RM estimate)")
    expect(bench).to_contain_text("1 week · 1 workout")
    shot(coach, "16a-starter-programs")
    # Cancel creates nothing
    bench.get_by_role("button", name="Use this program").click()
    coach.locator(".sheet").get_by_role("button", name="Cancel").click()
    expect(coach.locator(".sheet")).to_have_count(0)
    expect(coach.get_by_role("heading", name="Programs", exact=True)).to_be_visible()
    bench.get_by_role("button", name="Use this program").click()
    coach.locator(".sheet").get_by_role("button", name="Add to my programs").click()
    expect(coach.get_by_text("Added to your programs")).to_be_visible()
    expect(coach.get_by_label("Program name")).to_have_value("Strength benchmark (1RM estimate)")
    coach.locator(".cal .day.has").first.click()
    expect(coach.locator(".block-card", has_text="Stage 5 · Back Squat")).to_be_visible()
    shot(coach, "16b-benchmark-workout")

    # ---------------- recipes and meal plans: import, a shared book, a plan for today, logged
    coach.get_by_role("link", name="Programs").click()
    coach.locator(".subtabs").get_by_role("button", name="Recipes").click()
    expect(coach.get_by_role("heading", name="Recipes", exact=True)).to_be_visible()
    coach.get_by_role("button", name="+ New recipe book").click()
    coach.locator(".sheet").get_by_label("Name", exact=True).fill("Breakfasts")
    coach.locator(".sheet").get_by_role("button", name="Save book").click()
    expect(coach.get_by_text("Shared with all clients")).to_be_visible()
    coach.get_by_role("button", name="Import").click()
    coach.get_by_label("Recipe text").fill("Overnight oats\nCreamy oats soaked overnight.\nServes 2\nIngredients\n- 100 g oats\n- 200 g milk\nMethod\n1. Mix and chill overnight.")
    coach.get_by_role("button", name="Read recipe").click()
    expect(coach.get_by_label("Title", exact=True)).to_have_value("Overnight oats")
    coach.get_by_label("Calories of ingredient 1").fill("380")
    coach.get_by_label("Calories of ingredient 2").fill("120")
    expect(coach.locator(".card", has_text="Ingredients")).to_contain_text("Per serving: 250 kcal")
    coach.get_by_label("Book").select_option(label="Breakfasts")
    coach.get_by_role("button", name="Save recipe").click()
    expect(coach.get_by_text("Recipe saved")).to_be_visible()
    shot(coach, "16f-recipe")
    coach.get_by_role("button", name="‹ Recipes").click()
    coach.locator(".subtabs").get_by_role("button", name="Meal plans").click()
    expect(coach.get_by_role("heading", name="Meal plans", exact=True)).to_be_visible()
    coach.get_by_role("button", name="+ New", exact=True).click()
    coach.get_by_label("Name", exact=True).fill("Lean week")
    weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][datetime.date.today().weekday()]
    coach.get_by_role("button", name=f"+ Meal on {weekday}").click()
    coach.locator(".sheet").get_by_label("Meal", exact=True).select_option("Breakfast")
    coach.locator(".sheet").get_by_role("button", name="Add", exact=True).click()
    expect(coach.locator(".plan-day", has_text=weekday)).to_contain_text("250 kcal")
    coach.get_by_role("button", name="Save meal plan").click()
    expect(coach.get_by_text("Meal plan saved")).to_be_visible()
    coach.goto(coach.url.split("#")[0] + "#/c/client" + RUN + "/overview")
    coach.locator(".card", has_text="Food journal").get_by_role("button", name="Meal plan").click()
    coach.locator(".sheet").get_by_role("button", name="Assign").click()
    expect(coach.get_by_text("Meal plan assigned")).to_be_visible()
    # the client sees today's planned breakfast and logs it in a tap
    client.get_by_role("link", name="Today").click()
    plan = client.locator(".plan-today")
    expect(plan).to_contain_text("Breakfast: Overnight oats × 1")
    plan.get_by_role("button", name="Log").click()
    expect(client.get_by_text("Logged to your food journal")).to_be_visible()
    expect(client.locator(".meal-line", has_text="250 kcal")).to_contain_text("Breakfast")
    shot(client, "16g-todays-plan")
    # the shared book, and a PDF of their own read on the device
    client.locator(".card", has_text="Meals").get_by_role("link", name="Recipes").click()
    expect(client.locator(".card", has_text="Breakfasts")).to_contain_text("Overnight oats")
    expect(client.locator(".card", has_text="Breakfasts").locator(".recipe-desc")).to_have_text("Creamy oats soaked overnight.")  # descriptions show in the list
    client.get_by_role("button", name="Import").click()
    client.get_by_label("Import from").select_option("pdf")
    client.get_by_label("PDF file").set_input_files(files=[{"name": "smoothie.pdf", "mimeType": "application/pdf", "buffer": tiny_pdf(["Banana smoothie", "Serves 1", "Ingredients", "- 1 banana", "- 250 g milk", "Method", "1. Blend until smooth."])}])
    client.get_by_role("button", name="Read recipe").click()
    expect(client.get_by_label("Title", exact=True)).to_have_value("Banana smoothie")
    client.get_by_role("button", name="Save recipe").click()
    expect(client.get_by_text("Recipe saved")).to_be_visible()
    client.get_by_role("button", name="‹ Recipes").click()
    expect(client.locator(".card", has_text="My recipes")).to_contain_text("Banana smoothie")
    shot(client, "16h-client-recipes")

    # ---------------- service credits: a plan, required, granted; the client stops at zero
    coach.get_by_role("link", name="Profile").click()
    plans = coach.locator(".card", has=coach.get_by_role("heading", name="Plans & credits"))
    plans.get_by_role("button", name="+ New plan").click()
    sheet_ = coach.locator(".sheet")
    sheet_.get_by_label("Name", exact=True).fill("Monthly")
    sheet_.get_by_label("Description").fill("A month of coaching with weekly video feedback")
    sheet_.get_by_label("Price").fill("49")
    sheet_.get_by_label("Include Messages").check()
    sheet_.get_by_label("Messages credits").fill("1")
    sheet_.get_by_label("Include Video reviews").check()
    sheet_.get_by_label("Unlimited Video reviews").check()
    sheet_.get_by_role("button", name="Save plan").click()
    expect(plans).to_contain_text("$49.00 · 1 messages, Unlimited video reviews · for 30 days")
    plans.get_by_label("Require credits").check()
    coach.locator(".sheet").get_by_role("button", name="Require credits").click()
    expect(coach.get_by_text("Credits are now required")).to_be_visible()
    shot(coach, "16c-plans")
    coach.goto(coach.url.split("#")[0] + "#/c/client" + RUN + "/overview")
    credits_card = coach.locator(".card", has=coach.get_by_role("heading", name="Credits", exact=True))
    credits_card.get_by_role("button", name="Grant a plan").click()
    coach.locator(".sheet").get_by_role("button", name="Grant", exact=True).click()
    expect(credits_card).to_contain_text("1 left")
    shot(coach, "16d-client-credits")
    client.get_by_role("link", name="Coach").click()
    client.get_by_label("Message", exact=True).fill("One more question")
    client.get_by_role("button", name="Send", exact=True).click()
    expect(client.get_by_text("One more question")).to_be_visible()
    client.get_by_label("Message", exact=True).fill("And another")
    client.get_by_role("button", name="Send", exact=True).click()
    expect(client.get_by_text("You have no messages left")).to_be_visible()
    client.get_by_role("link", name="Profile").click()
    expect(client.locator(".card", has_text="Your plan")).to_contain_text("None left")
    shot(client, "16e-your-plan")
    # off again for the rest of the journey
    coach.get_by_role("link", name="Profile").click()
    plans.get_by_label("Require credits").uncheck()
    expect(coach.get_by_text("Credits are no longer required")).to_be_visible()
    coach.goto(coach.url.split("#")[0] + "#/")

    # ---------------- assistant coaches: a nutrition coach on the head coach's team
    coach.locator('#tabs a[data-path="/me"]').click()
    team = coach.locator(".card", has=coach.get_by_role("heading", name="Your team"))
    team.get_by_role("button", name="+ Invite assistant").click()
    inv = coach.locator(".sheet")
    inv.get_by_label("Email").fill(f"asst{RUN}@dev.local")
    inv.get_by_label("Name", exact=True).fill("Maria")
    inv.get_by_label("Title").fill("Nutrition Coach")
    inv.get_by_label("Nutrition", exact=True).check()
    inv.get_by_label("Messaging", exact=True).check()
    inv.get_by_label("Progress (view)").uncheck()
    inv.get_by_role("button", name="Send invitation").click()
    link = inv.get_by_label("Invitation link").input_value()
    inv.get_by_role("button", name="Close").click()
    expect(team).to_contain_text(f"Invited asst{RUN}@dev.local as Nutrition Coach")
    asst_ctx = browser.new_context(**mobile)
    asst = asst_ctx.new_page(); watch(asst, "assistant")
    sign_in(asst, "asst" + RUN, BASE + "#/join/" + link.split("#/join/")[1])
    expect(asst.get_by_text(re.compile("to their coaching team as Nutrition Coach"))).to_be_visible()
    asst.get_by_label("I agree to how PT Coach uses my data").check()
    asst.get_by_role("button", name="Accept invite").click()
    expect(asst.get_by_text("No clients are assigned to you yet")).to_be_visible()
    # the tabs follow the permissions: messages and recipes, no videos or exercises
    expect(asst.locator('#tabs a[data-path="/messages"]')).to_be_visible()
    expect(asst.locator('#tabs a[data-path="/recipes"]')).to_be_visible()
    expect(asst.locator('#tabs a[data-path="/videos"]')).to_have_count(0)
    expect(asst.locator('#tabs a[data-path="/exercises"]')).to_have_count(0)
    # the head coach assigns Sam
    coach.reload()
    team.get_by_role("button", name="Edit").click()
    coach.locator(".sheet").get_by_label("Assign Sam Client").check()
    coach.locator(".sheet").get_by_role("button", name="Save", exact=True).click()
    expect(team).to_contain_text("Clients: Sam Client")
    shot(coach, "17-team")
    asst.reload()
    asst.locator(".card.link", has_text="Sam Client").click()
    expect(asst.get_by_role("heading", name="Food journal")).to_be_visible()
    expect(asst.locator(".subtabs").get_by_role("button", name="Intake")).to_have_count(0)  # no health information
    asst.locator(".subtabs").get_by_role("button", name="Messages").click()
    asst.get_by_label("Message", exact=True).fill("Nice work on your protein this week!")
    asst.get_by_role("button", name="Send", exact=True).click()
    expect(asst.get_by_text("Nice work on your protein this week!")).to_be_visible()
    shot(asst, "17a-assistant")
    # the client sees who wrote
    client.locator('#tabs a[data-path="/coach"]').click()
    expect(client.get_by_text("Maria · Nutrition Coach").first).to_be_visible(timeout=20000)
    # removed from the team: their access goes at once
    team.get_by_role("button", name="Edit").click()
    coach.locator(".sheet").get_by_role("button", name="Remove from team").click()
    coach.get_by_role("button", name="Remove", exact=True).click()
    expect(coach.get_by_text("Maria removed")).to_be_visible()
    expect(team).not_to_contain_text("Nutrition Coach")
    asst_ctx.close()
    coach.goto(coach.url.split("#")[0] + "#/")

    # ---------------- the admin suspends the coach, then lifts it
    admin.reload()
    admin.locator(".card.link", has_text=f"coach{RUN}@dev.local").click()
    expect(admin.locator(".sheet")).to_contain_text("Clients (1)")
    expect(admin.locator(".sheet").get_by_role("button", name="Move…")).to_be_visible()
    expect(admin.locator(".sheet")).to_contain_text("Coaching time (last 4 weeks)")
    admin.locator(".sheet").get_by_role("button", name="Suspend coach").click()
    admin.get_by_role("button", name="Suspend", exact=True).click()
    expect(admin.locator(".card.link", has_text=f"coach{RUN}@dev.local").get_by_text("Suspended")).to_be_visible()
    coach.reload()
    expect(coach.get_by_role("heading", name="Account suspended")).to_be_visible()
    shot(coach, "15-coach-suspended")
    admin.locator(".card.link", has_text=f"coach{RUN}@dev.local").click()
    admin.locator(".sheet").get_by_role("button", name="Lift suspension").click()
    expect(admin.get_by_text("Suspension lifted")).to_be_visible()
    coach.reload()
    expect(coach.get_by_role("link", name="Messages")).to_be_visible()

    # ---------------- the header's sign-out button
    admin.get_by_role("button", name="Sign out").click()
    expect(admin.get_by_role("button", name="Sign in", exact=True)).to_be_visible()
    expect(admin.get_by_role("button", name="Sign out")).to_be_hidden()

    # ---------------- a used invitation link stops working
    stale_ctx = browser.new_context(**mobile)
    stale = stale_ctx.new_page()  # not watched: the lookup 404s on purpose
    stale.goto(BASE + "#/join/" + coach_code)
    expect(stale.get_by_role("heading", name="This link doesn’t work")).to_be_visible()

    # ---------------- the client downloads their data, then deletes their account
    client.get_by_role("link", name="Profile").click()
    with client.expect_download() as dl:
        client.get_by_role("button", name="Download my data").click()
    assert dl.value.suggested_filename.startswith("pt-coach-data-"), dl.value.suggested_filename
    client.get_by_role("button", name="Delete my account").click()
    client.locator(".sheet").get_by_label("Type your email to confirm").fill(f"client{RUN}@dev.local")
    client.locator(".sheet").get_by_role("button", name="Delete my account").click()
    expect(client.get_by_text("Your account and everything in it have been deleted.")).to_be_visible()

    browser.close()

print("\nERRORS:" if errors else "\nno console errors")
for e in errors:
    print(" ", e)
sys.exit(1 if errors else 0)
