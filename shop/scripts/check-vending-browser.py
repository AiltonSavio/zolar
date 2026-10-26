#!/usr/bin/env python3
"""Exercise the dev-only vending fixture with one agent-browser session."""
import argparse
import json
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--url', default='http://127.0.0.1:5188')
parser.add_argument('--out', default='.zolar-work/screenshots')
args = parser.parse_args()
out = Path(args.out).resolve()
out.mkdir(parents=True, exist_ok=True)


def browser(*arguments):
    result = subprocess.run(['agent-browser', '--session', 'zolar-vending-shop', *arguments], check=True, text=True, capture_output=True)
    return result.stdout.strip()


def wait_for(expression):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if browser('eval', expression) == 'true':
            return
        time.sleep(0.1)
    raise AssertionError(f'Timed out: {expression}')


states = {
    'connect': 'Get your ticket', 'reserve': 'Get your ticket',
    'payment': 'Pay with Zcash', 'finality': 'Waiting for finality',
    'proving': 'Making your payment proof…', 'claiming': 'Sending your claim…',
    'success': 'You’re in.', 'sold-out': 'Sold out', 'expired': 'Reservation expired',
    'halted': 'The relay is halted. Do not pay.',
    'stalled': 'The relay has stopped advancing. Do not pay.',
    'proof-failed': 'Proof failed', 'claim-failed': 'Claim failed',
}
started = time.monotonic()
try:
    for state, expected in states.items():
        browser('open', f'{args.url}/fixture.html?state={state}')
        wait_for('!!document.querySelector("button")')
        if state not in ['connect', 'sold-out', 'halted', 'stalled']:
            browser('click', 'button:has-text("Connect Solana wallet")')
            wait_for('!![...document.querySelectorAll("button")].find(b => b.textContent === "Reserve ticket" && !b.disabled)')
            if state != 'reserve':
                browser('click', 'button:has-text("Reserve ticket")')
                if state == 'claim-failed':
                    wait_for('!![...document.querySelectorAll("button")].find(b => b.textContent === "Check claim")')
                    browser('click', 'button:has-text("Check claim")')
        wait_for(f'document.body.innerText.includes({json.dumps(expected)})')
        if state == 'payment':
            wait_for('!!document.querySelector(".payment-qr")')
            browser('click', 'button:has-text("Copy payment link")')
            wait_for('document.body.innerText.includes("Copied") || document.body.innerText.includes("Could not copy. Select the payment link below.")')
            assert browser('eval', 'document.querySelector(".payment-open").href === document.querySelector(".payment-link").value') == 'true'
        if state in ['sold-out', 'halted', 'stalled', 'expired', 'proof-failed', 'claim-failed']:
            assert browser('eval', '!document.querySelector(".payment-open")') == 'true'
        for width, height, suffix in [(1440, 1100, ''), (390, 844, '-phone')]:
            browser('set', 'viewport', str(width), str(height))
            assert browser('eval', 'document.documentElement.scrollWidth <= innerWidth') == 'true'
            browser('screenshot', str(out / f'{state}{suffix}.png'), '--full')
        if state == 'expired':
            browser('click', 'button:has-text("Release reservation")')
            wait_for('!![...document.querySelectorAll("button")].find(b => b.textContent === "Reserve ticket")')
        assert not browser('errors'), f'Browser errors in {state}'
        print(f'PASS {state}: {args.url}/fixture.html?state={state}')
    print(f'PASS {len(states)} states, {len(states) * 2} screenshots, {time.monotonic() - started:.1f}s')
finally:
    browser('close')
