import time
import json
import logging
import random
import sys
import os
import shutil
import platform
from datetime import datetime, time as dt_time, date, timedelta
from contextlib import contextmanager

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.common.keys import Keys
from selenium.common.exceptions import (
    NoSuchElementException, TimeoutException,
    StaleElementReferenceException, ElementNotInteractableException
)
from selenium.webdriver.chrome.service import Service
from selenium.webdriver.chrome.options import Options

os.environ['WDM_LOG'] = '0'
os.environ['WDM_PROGRESS_BAR'] = '0'
from webdriver_manager.chrome import ChromeDriverManager

# ──────────────────────────────────────────────
# CONFIG
# ──────────────────────────────────────────────
CONFIG_FILE = "config.json"

DEFAULT_CONFIG = {
    "TEST_MODE": False,
    "TARGET_USERS": ["kullanici1", "kullanici2"],
    "MESSAGE_TO_SEND": ".",
    "TARGET_SEND_TIME_HM": [0, 2],
    "COOKIES_FILE": "cookies.json",
    "LOG_FILENAME": "tiktok_bot.txt",
    "USER_AGENT": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/117.0.0.0 Safari/537.36",
    "TIKTOK_MESSAGES_URL": "https://www.tiktok.com/messages?lang=en",
    "HEADLESS_MODE": True
}

IS_LINUX = platform.system() == "Linux"
IS_WINDOWS = platform.system() == "Windows"

# ──────────────────────────────────────────────
# LOGGING (early init, will be reconfigured after config load)
# ──────────────────────────────────────────────
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s',
    handlers=[logging.StreamHandler()]
)

# ──────────────────────────────────────────────
# CONFIG LOADER
# ──────────────────────────────────────────────
def load_or_create_config(filename):
    if not os.path.exists(filename):
        logging.warning(f"Config '{filename}' not found. Creating with defaults.")
        try:
            with open(filename, 'w', encoding='utf-8') as f:
                json.dump(DEFAULT_CONFIG, f, indent=4, ensure_ascii=False)
            logging.info(f"Default config '{filename}' created.")
            return DEFAULT_CONFIG
        except Exception as e:
            logging.error(f"Could not create config: {e}")
            return None
    else:
        try:
            with open(filename, 'r', encoding='utf-8') as f:
                data = json.load(f)
            logging.info(f"Config loaded from '{filename}'.")
            return data
        except json.JSONDecodeError as e:
            logging.error(f"Invalid JSON in config: {e}")
            return None
        except Exception as e:
            logging.error(f"Error loading config: {e}")
            return None

# ──────────────────────────────────────────────
# PROCESS CLEANUP
# ──────────────────────────────────────────────
def terminate_lingering_processes():
    logging.info("Terminating any lingering chrome/chromedriver processes...")
    try:
        if IS_WINDOWS:
            os.system("taskkill /F /IM chromedriver.exe /T > NUL 2>&1")
            os.system("taskkill /F /IM chrome.exe /T > NUL 2>&1")
        else:
            os.system("pkill -f chromedriver > /dev/null 2>&1")
            os.system("pkill -f chromium > /dev/null 2>&1")
            os.system("pkill -f chrome > /dev/null 2>&1")
        logging.info("Process cleanup done.")
    except Exception as e:
        logging.error(f"Error during process cleanup: {e}")

# ──────────────────────────────────────────────
# LOAD CONFIG
# ──────────────────────────────────────────────
config = load_or_create_config(CONFIG_FILE)
if config is None:
    logging.critical("Exiting: config error.")
    sys.exit(1)

TEST_MODE        = config.get('TEST_MODE',            DEFAULT_CONFIG['TEST_MODE'])
TARGET_USERS     = config.get('TARGET_USERS',         DEFAULT_CONFIG['TARGET_USERS'])
MESSAGE_TO_SEND  = config.get('MESSAGE_TO_SEND',      DEFAULT_CONFIG['MESSAGE_TO_SEND'])
time_hm          = config.get('TARGET_SEND_TIME_HM',  DEFAULT_CONFIG['TARGET_SEND_TIME_HM'])
COOKIES_FILE     = config.get('COOKIES_FILE',         DEFAULT_CONFIG['COOKIES_FILE'])
LOG_FILENAME     = config.get('LOG_FILENAME',         DEFAULT_CONFIG['LOG_FILENAME'])
USER_AGENT       = config.get('USER_AGENT',           DEFAULT_CONFIG['USER_AGENT'])
TIKTOK_MESSAGES_URL = config.get('TIKTOK_MESSAGES_URL', DEFAULT_CONFIG['TIKTOK_MESSAGES_URL'])
HEADLESS_MODE    = True  # Always True on Linux VPS (no display)

# Ensure cookies file exists
if not os.path.exists(COOKIES_FILE):
    try:
        with open(COOKIES_FILE, 'w', encoding='utf-8') as f:
            json.dump([], f, indent=4)
        logging.info(f"Empty cookies file created: '{COOKIES_FILE}'.")
    except Exception as e:
        logging.error(f"Could not create cookies file: {e}")

# Parse target time
try:
    if isinstance(time_hm, list) and len(time_hm) == 2:
        TARGET_SEND_TIME = dt_time(int(time_hm[0]), int(time_hm[1]))
    else:
        raise ValueError("TARGET_SEND_TIME_HM must be [hour, minute]")
except (ValueError, TypeError) as e:
    logging.error(f"Invalid TARGET_SEND_TIME_HM: {e}. Using default.")
    TARGET_SEND_TIME = dt_time(DEFAULT_CONFIG['TARGET_SEND_TIME_HM'][0],
                               DEFAULT_CONFIG['TARGET_SEND_TIME_HM'][1])

# Reconfigure logging with file handler
for h in logging.root.handlers[:]:
    logging.root.removeHandler(h)
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s',
    handlers=[
        logging.FileHandler(LOG_FILENAME, encoding='utf-8'),
        logging.StreamHandler()
    ]
)

logging.info("--- Bot Started ---")
logging.info(f"Platform: {platform.system()} | TEST_MODE: {TEST_MODE} | Target Time: {TARGET_SEND_TIME.strftime('%H:%M')}")

# ──────────────────────────────────────────────
# XPATHS
# ──────────────────────────────────────────────
MESSAGE_LIST_CONTAINER_XPATH = "//*[@data-e2e='dm-new-conversation-list']"
CONVERSATION_ITEM_XPATH      = "//*[@data-e2e='dm-new-conversation-item']"
NICKNAME_CLASS_PARTIAL       = "PInfoNickname"
NICKNAME_XPATH_INSIDE_ITEM   = f".//p[contains(@class, '{NICKNAME_CLASS_PARTIAL}')]"
CLICK_TARGET_XPATH           = '//*[@id="main-content-messages"]/div/div[3]/div[4]/div'
WRITE_TARGET_XPATH           = '//*[@id="main-content-messages"]/div/div[3]/div[4]/div/div[1]/div/div[2]/div[2]/div/div/div/div'
TOAST_XPATH                  = "//li[@data-sonner-toast]"

# ──────────────────────────────────────────────
# COOKIE LOADER
# ──────────────────────────────────────────────
def load_cookies(driver, cookie_file):
    logging.info(f"Loading cookies from '{cookie_file}'...")
    added = 0
    failed = 0
    try:
        with open(cookie_file, 'r') as f:
            cookies = json.load(f)
        logging.info(f"Read {len(cookies)} cookies.")

        driver.get("https://www.tiktok.com/explore")
        logging.info("Navigated to tiktok.com. Waiting before injecting cookies...")
        time.sleep(random.uniform(3, 5))

        for i, cookie in enumerate(cookies):
            c = {}
            try:
                c['name']  = cookie['name']
                c['value'] = cookie['value']
                for field in ('path', 'domain', 'secure', 'httpOnly'):
                    if field in cookie:
                        c[field] = cookie[field]

                if cookie.get('expirationDate'):
                    try:
                        c['expiry'] = int(float(cookie['expirationDate']))
                    except (ValueError, TypeError):
                        pass

                ss = cookie.get('sameSite')
                if ss is None or (isinstance(ss, str) and ss.lower() == 'no_restriction'):
                    if c.get('secure'):
                        c['sameSite'] = 'None'
                elif isinstance(ss, str) and ss.lower() in ('lax', 'strict', 'none'):
                    c['sameSite'] = ss.capitalize()

                if not c.get('domain'):
                    c['domain'] = ".tiktok.com"

                driver.add_cookie(c)
                added += 1
            except Exception as e:
                failed += 1
                logging.warning(f"Cookie #{i+1} ('{cookie.get('name', '?')}') failed: {type(e).__name__}")

        if failed:
            logging.warning(f"{failed} cookies failed.")
        if added:
            logging.info(f"Successfully added {added} cookies.")
            return True
        else:
            logging.error("No cookies were added!")
            return False

    except FileNotFoundError:
        logging.error(f"Cookie file not found: {cookie_file}")
        return False
    except json.JSONDecodeError:
        logging.error(f"Cookie file is not valid JSON: {cookie_file}")
        return False
    except Exception as e:
        logging.error(f"Unexpected error loading cookies: {e}")
        return False

# ──────────────────────────────────────────────
# HELPERS
# ──────────────────────────────────────────────
def wait_for_element(driver, by, value, timeout=20):
    try:
        WebDriverWait(driver, timeout).until(EC.presence_of_element_located((by, value)))
        logging.info(f"Element found: {value}")
        return True
    except TimeoutException:
        logging.error(f"Timeout waiting for element: {value}")
        return False

def find_and_click_conversation(driver, username):
    logging.info(f"Searching for conversation: '{username}'...")
    try:
        WebDriverWait(driver, 30).until(
            EC.presence_of_element_located((By.XPATH, CONVERSATION_ITEM_XPATH))
        )
        time.sleep(random.uniform(2, 4))

        items = driver.find_elements(By.XPATH, CONVERSATION_ITEM_XPATH)
        logging.info(f"Found {len(items)} conversation items.")

        if not items:
            logging.warning("No conversation items found.")
            return False

        for i, item in enumerate(items):
            try:
                nick_el = item.find_element(By.XPATH, NICKNAME_XPATH_INSIDE_ITEM)
                nick    = nick_el.text.strip()
                if nick.lower() == username.lower():
                    logging.info(f"Found '{username}' at item #{i+1}. Clicking...")
                    driver.execute_script("arguments[0].scrollIntoView(true);", item)
                    time.sleep(0.5)
                    item.click()
                    time.sleep(random.uniform(3, 5))
                    return True
            except NoSuchElementException:
                continue
            except StaleElementReferenceException:
                logging.warning(f"Stale element at item #{i+1}, skipping.")
                continue
            except Exception as e:
                logging.error(f"Error at item #{i+1}: {e}")
                continue

        logging.warning(f"'{username}' not found in {len(items)} items.")
        return False

    except TimeoutException:
        logging.error("Timeout waiting for conversation items.")
        return False
    except Exception as e:
        logging.error(f"Unexpected error in find_and_click_conversation: {e}")
        return False

def send_message_in_open_chat(driver):
    logging.info("Sending message...")
    try:
        # Wait for any toast to disappear
        try:
            WebDriverWait(driver, 7).until(
                EC.invisibility_of_element_located((By.XPATH, TOAST_XPATH))
            )
        except TimeoutException:
            pass

        # Click chat area
        try:
            click_target = WebDriverWait(driver, 15).until(
                EC.element_to_be_clickable((By.XPATH, CLICK_TARGET_XPATH))
            )
            driver.execute_script("arguments[0].scrollIntoView(true);", click_target)
            time.sleep(0.5)
            click_target.click()
            time.sleep(random.uniform(1.5, 2.5))
        except TimeoutException:
            logging.error("Click target not found.")
            return False
        except Exception as e:
            logging.warning(f"Normal click failed: {e}. Trying JS click...")
            try:
                driver.execute_script("arguments[0].click();", click_target)
                time.sleep(random.uniform(1.5, 2.5))
            except Exception as je:
                logging.error(f"JS click also failed: {je}")
                return False

        # Find write area
        try:
            write_target = WebDriverWait(driver, 15).until(
                EC.visibility_of_element_located((By.XPATH, WRITE_TARGET_XPATH))
            )
            driver.execute_script("arguments[0].focus();", write_target)
            time.sleep(0.5)
        except TimeoutException:
            logging.error("Write target not found.")
            return False

        # Send message
        try:
            WebDriverWait(driver, 5).until(EC.element_to_be_clickable(write_target))
            write_target.send_keys(MESSAGE_TO_SEND)
            time.sleep(random.uniform(0.8, 1.5))
            write_target.send_keys(Keys.ENTER)
            logging.info("Message sent.")
            time.sleep(random.uniform(2, 4))
            return True
        except ElementNotInteractableException:
            logging.warning("send_keys failed. Trying JS textContent...")
            try:
                driver.execute_script(
                    "arguments[0].textContent = arguments[1];", write_target, MESSAGE_TO_SEND
                )
                time.sleep(random.uniform(0.5, 1.0))
                write_target.send_keys(Keys.ENTER)
                logging.info("Message sent via JS.")
                time.sleep(random.uniform(2, 4))
                return True
            except Exception as je:
                logging.error(f"JS send also failed: {je}")
                return False
        except Exception as e:
            logging.error(f"Error sending message: {e}")
            return False

    except Exception as e:
        logging.error(f"General error in send_message: {e}")
        return False

def handle_passkey_popup(driver):
    logging.info("Checking for passkey popup...")
    xpath = "//div[starts-with(@id, 'floating-ui-')]/div/div[2]/button[1]"
    try:
        btn = WebDriverWait(driver, 15).until(EC.element_to_be_clickable((By.XPATH, xpath)))
        logging.info("Passkey popup found. Dismissing...")
        btn.click()
        WebDriverWait(driver, 10).until(EC.invisibility_of_element_located((By.XPATH, xpath)))
        logging.info("Passkey popup dismissed.")
    except TimeoutException:
        logging.info("No passkey popup.")
    except Exception as e:
        logging.warning(f"Passkey popup error: {e}")

# ──────────────────────────────────────────────
# WEBDRIVER — optimized for Linux VPS 1GB RAM
# ──────────────────────────────────────────────
def get_chrome_binary():
    """Find Chrome/Chromium binary on Linux."""
    candidates = [
        "/usr/bin/chromium-browser",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/snap/bin/chromium",
    ]
    for path in candidates:
        if os.path.exists(path):
            logging.info(f"Found Chrome binary: {path}")
            return path
    return None  # Let ChromeDriver find it automatically

@contextmanager
def managed_webdriver():
    terminate_lingering_processes()
    time.sleep(1)

    SESSION_DIR = os.path.join(os.getcwd(), "session_data")
    if os.path.exists(SESSION_DIR):
        try:
            shutil.rmtree(SESSION_DIR, ignore_errors=True)
            logging.info("Previous session directory cleaned.")
            time.sleep(1)
        except Exception as e:
            logging.warning(f"Could not clean session dir: {e}")

    driver = None
    try:
        opts = Options()

        # ── Headless (always on Linux, config-based on Windows) ──
        if IS_LINUX or HEADLESS_MODE:
            opts.add_argument("--headless=new")
            logging.info("Headless mode: ON")

        # ── Anti-detection ──
        opts.add_argument(f"user-agent={USER_AGENT}")
        opts.add_argument("--disable-blink-features=AutomationControlled")
        opts.add_experimental_option("excludeSwitches", ["enable-automation"])
        opts.add_experimental_option("useAutomationExtension", False)

        # ── Linux VPS required flags ──
        opts.add_argument("--no-sandbox")           # Required in containers/VPS
        opts.add_argument("--disable-dev-shm-usage") # Prevents /dev/shm OOM on 1GB RAM
        opts.add_argument("--disable-gpu")
        opts.add_argument("--disable-software-rasterizer")

        # ── RAM optimization ──
        opts.add_argument("--single-process")        # Reduces memory significantly
        opts.add_argument("--no-zygote")             # Works with single-process
        opts.add_argument("--renderer-process-limit=1")
        opts.add_argument("--disable-extensions")
        opts.add_argument("--disable-plugins")
        opts.add_argument("--disable-images")        # Don't load images (saves RAM + bandwidth)
        opts.add_argument("--blink-settings=imagesEnabled=false")
        opts.add_argument("--disable-javascript-harmony-shipping")
        opts.add_argument("--disable-background-networking")
        opts.add_argument("--disable-default-apps")
        opts.add_argument("--disable-sync")
        opts.add_argument("--disable-translate")
        opts.add_argument("--hide-scrollbars")
        opts.add_argument("--metrics-recording-only")
        opts.add_argument("--mute-audio")
        opts.add_argument("--no-first-run")
        opts.add_argument("--safebrowsing-disable-auto-update")
        opts.add_argument("--log-level=3")
        opts.add_argument("--silent")

        # ── Window size (needed for headless element detection) ──
        opts.add_argument("--window-size=1280,800")

        # ── Session directory ──
        opts.add_argument(f"--user-data-dir={SESSION_DIR}")

        # ── Chrome binary (Linux) ──
        if IS_LINUX:
            binary = get_chrome_binary()
            if binary:
                opts.binary_location = binary

        service = Service(ChromeDriverManager().install())
        driver = webdriver.Chrome(service=service, options=opts)

        # Reduce page load timeout to avoid hanging
        driver.set_page_load_timeout(60)
        driver.set_script_timeout(30)

        yield driver

    finally:
        logging.info("Cleanup phase...")
        if driver:
            try:
                driver.quit()
                time.sleep(2)
            except Exception as e:
                logging.warning(f"driver.quit() error: {e}")

        terminate_lingering_processes()

        try:
            time.sleep(2)
            shutil.rmtree(SESSION_DIR, ignore_errors=True)
            logging.info("Session directory cleaned up.")
        except Exception as e:
            logging.error(f"Failed to remove session dir: {e}")

# ──────────────────────────────────────────────
# MAIN BOT LOGIC
# ──────────────────────────────────────────────
def run_bot():
    if TEST_MODE:
        logging.warning("--- TEST MODE ACTIVE ---")
    else:
        logging.info("--- Normal Mode ---")

    try:
        with managed_webdriver() as driver:
            logging.info("Browser ready.")

            if not load_cookies(driver, COOKIES_FILE):
                raise Exception("Failed to load cookies.")

            logging.info(f"Navigating to {TIKTOK_MESSAGES_URL}...")
            driver.get(TIKTOK_MESSAGES_URL)

            handle_passkey_popup(driver)

            logging.info("Waiting for message list...")
            if not wait_for_element(driver, By.XPATH, MESSAGE_LIST_CONTAINER_XPATH, timeout=35):
                logging.warning("Message list container not found. Continuing anyway...")
            else:
                logging.info("Message list loaded.")

            time.sleep(random.uniform(3, 6))

            if not TARGET_USERS:
                logging.error("TARGET_USERS is empty. Nothing to do.")
                return

            success = 0
            logging.info(f"Targeting {len(TARGET_USERS)} user(s): {', '.join(TARGET_USERS)}")

            for user in TARGET_USERS:
                safe_user = ''.join(c for c in user if c.isprintable())
                logging.info(f"--- Processing: '{safe_user}' ---")

                if find_and_click_conversation(driver, user):
                    if send_message_in_open_chat(driver):
                        success += 1
                        logging.info(f"✓ Message sent to '{safe_user}'.")
                    else:
                        logging.warning(f"✗ Opened chat for '{safe_user}' but failed to send.")
                else:
                    logging.warning(f"✗ Conversation not found for '{safe_user}'.")

                if len(TARGET_USERS) > 1 and user != TARGET_USERS[-1]:
                    wait = random.uniform(5, 10)
                    logging.info(f"Waiting {wait:.1f}s before next user...")
                    time.sleep(wait)

            logging.info(f"Done. {success}/{len(TARGET_USERS)} messages sent.")

    except Exception as e:
        logging.error(f"Critical error in run_bot: {e}")
        logging.exception(e)

# ──────────────────────────────────────────────
# ENTRY POINT
# ──────────────────────────────────────────────
if __name__ == "__main__":
    if TEST_MODE:
        logging.info("Running immediately (TEST_MODE).")
        run_bot()
        logging.info("Test run finished.")
    else:
        last_run_date = None
        logging.info(f"Scheduled mode. Will run daily at {TARGET_SEND_TIME.strftime('%H:%M')}.")

        while True:
            now          = datetime.now()
            current_time = now.time()
            today        = now.date()

            window_end   = (datetime.combine(today, TARGET_SEND_TIME) + timedelta(minutes=1)).time()
            in_window    = TARGET_SEND_TIME <= current_time < window_end

            if in_window and today != last_run_date:
                logging.info(f"Target time reached. Running bot...")
                try:
                    run_bot()
                except Exception as e:
                    logging.error(f"Unhandled exception in run_bot: {e}")
                finally:
                    last_run_date = today
                    logging.info(f"Run complete. Next run: tomorrow {TARGET_SEND_TIME.strftime('%H:%M')}.")

            # Dynamic sleep: check every 5s near target time, every 60s otherwise
            next_run = datetime.combine(today, TARGET_SEND_TIME)
            if datetime.now() >= next_run:
                next_run += timedelta(days=1)

            secs_until = (next_run - datetime.now()).total_seconds()
            interval   = 5 if secs_until < 300 else 60

            logging.debug(f"Next run in {secs_until/3600:.1f}h. Checking again in {interval}s.")
            time.sleep(interval)
