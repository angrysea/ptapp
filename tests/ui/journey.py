import os, re, sys, time, datetime
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

BASE = os.environ.get("BASE_URL", "http://localhost:8080/")
OUT = Path(__file__).parent / "shots"
OUT.mkdir(exist_ok=True)
errors = []
RUN = str(int(time.time()))

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
    page.once("dialog", lambda d: d.accept(user))
    page.get_by_role("button", name="Sign in or create account").click()
    page.wait_for_load_state("networkidle")

with sync_playwright() as p:
    browser = p.chromium.launch(channel=os.environ.get("BROWSER_CHANNEL", "msedge"), headless=True)
    mobile = dict(viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True)

    # ---------------- coach signs up
    coach_ctx = browser.new_context(**mobile)
    coach = coach_ctx.new_page(); watch(coach, "coach")
    sign_in(coach, "coach" + RUN)
    coach.get_by_label("Your name").fill("Alex Coach")
    coach.get_by_role("button", name="Create coach account").click()
    expect(coach.get_by_role("heading", name="Clients")).to_be_visible()

    # ---------------- invite
    coach.get_by_role("button", name="+ Invite").click()
    coach.get_by_label("Client name").fill("Sam Client")
    coach.get_by_label("Email").fill("sam@example.com")
    coach.get_by_role("button", name="Create invite").click()
    expect(coach.get_by_role("heading", name="Invite link")).to_be_visible()
    link = coach.locator(".sheet input").input_value()
    code = link.rsplit("/", 1)[-1]
    print("invite", code)
    shot(coach, "01-coach-invite")
    coach.get_by_role("button", name="Done").click()

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
    coach.locator(".item-card").first.get_by_label("Load (kg)").fill("100")
    coach.locator(".item-card").first.get_by_label("Load (kg)").press("Tab")
    coach.locator(".item-card").first.get_by_label("Reps").fill("5")
    coach.locator(".item-card").first.get_by_label("Reps").press("Tab")
    add_block(coach, "emom")
    coach.locator(".block-card").nth(1).locator("select").nth(1).select_option(label="Burpee")
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
    expect(client.get_by_role("heading", name=re.compile("Join Alex Coach"))).to_be_visible()
    client.get_by_role("button", name="Accept invite").click()
    expect(client.get_by_role("heading", name=re.compile("Step 1 of 2"))).to_be_visible()
    client.get_by_label("Goals").fill("Squat 140 kg by summer")
    client.get_by_label("Sex (for body-fat equations)").select_option("male")
    client.get_by_label("Birth date").fill("1994-03-02")
    client.get_by_label("Height (cm)").fill("180")
    client.get_by_role("button", name="Next").click()
    expect(client.get_by_role("heading", name=re.compile("Step 2 of 2"))).to_be_visible()
    for r in client.get_by_label("No", exact=True).all():
        r.check()
    client.get_by_label("Signature").fill("Sam Client")
    client.get_by_label("I have read and agree to the waiver").check()
    shot(client, "04-waiver")
    client.get_by_role("button", name="Sign").click()
    expect(client.get_by_role("heading", name="Today")).to_be_visible()

    # ---------------- coach assigns the program starting this Monday
    monday = datetime.date.today() - datetime.timedelta(days=today_dow)
    coach.get_by_role("link", name="Clients").click()
    coach.get_by_text("Sam Client").first.click()
    coach.get_by_role("button", name="Calendar").click()
    coach.get_by_label("Start date").fill(monday.isoformat())
    coach.get_by_role("button", name="Assign").click()
    expect(coach.get_by_text("Program assigned")).to_be_visible()
    shot(coach, "05-coach-calendar")

    # ---------------- client logs today's workout, offline
    client.reload(); client.wait_for_load_state("networkidle")
    client.locator(".card.today").first.click()
    expect(client.get_by_role("heading", name=re.compile("Week 1"))).to_be_visible()
    client_ctx.set_offline(True)
    done = client.locator(".setrow .done")
    print("set rows", done.count())
    done.nth(0).click()
    client.get_by_role("button", name="Skip", exact=True).click()  # dismiss rest timer
    done.nth(1).click(); client.get_by_role("button", name="Skip", exact=True).click()
    client.locator(".setrow").nth(2).locator("input").first.fill("4")
    client.locator(".setrow").nth(2).locator("input").first.press("Tab")
    done.nth(2).click(); client.get_by_role("button", name="Skip", exact=True).click()
    client.get_by_role("button", name="⏱ Timer").click()
    time.sleep(1.2)
    shot(client, "06-emom-timer")
    client.locator(".sheet").get_by_role("button", name="Close").click()
    shot(client, "07-logger")
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

    # ---------------- client body metrics
    client.get_by_role("link", name="Progress").click()
    for d, w in [(-14, 84.0), (-7, 83.2), (0, 82.5)]:
        client.get_by_label("Date").fill((datetime.date.today() + datetime.timedelta(days=d)).isoformat())
        client.get_by_label("Weight (kg)").fill(str(w))
        client.get_by_role("button", name="Log weight").click()
        expect(client.get_by_text("Saved").first).to_be_visible()
        client.wait_for_timeout(300)
    shot(client, "09-weight")
    client.get_by_role("button", name="Body fat").click()
    inputs = client.locator(".sheet, .card").first.locator("input[placeholder='mm']")
    for i, v in enumerate([15, 25, 20]):
        inputs.nth(i).fill(str(v))
    client.get_by_role("button", name="Calculate & save").click()
    expect(client.get_by_text("Body fat").nth(1)).to_be_visible()
    shot(client, "10-skinfold")

    # ---------------- coach dashboard + feedback
    coach.get_by_role("button", name="Overview").click()
    print("coach url", coach.url)
    expect(coach.get_by_text("Weekly volume (kg × reps)")).to_be_visible()
    shot(coach, "11-coach-overview")
    coach.get_by_role("button", name="Logs").click()
    coach.once("dialog", lambda d: d.accept("Great depth. Keep the chest up on set 3"))
    coach.locator(".exercise button").nth(2).click()
    expect(coach.get_by_text(re.compile("Keep the chest up"))).to_be_visible()
    shot(coach, "12-coach-logs")
    coach.get_by_role("link", name="Clients").click()
    expect(coach.get_by_text("Pain reported")).to_be_visible()
    shot(coach, "13-roster")

    client.get_by_role("link", name="Coach").click()
    expect(client.get_by_text(re.compile("Keep the chest up"))).to_be_visible()
    shot(client, "14-client-messages")

    browser.close()

print("\nERRORS:" if errors else "\nno console errors")
for e in errors:
    print(" ", e)
sys.exit(1 if errors else 0)
