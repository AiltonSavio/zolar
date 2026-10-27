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
    'release-window': 'Release in', 'release-ready': 'Reservation expired',
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
            assert browser('eval', '!document.body.innerText.includes("Could not copy")') == 'true'
            assert browser('eval', 'document.querySelector(".payment-open").href === document.querySelector(".payment-link").value') == 'true'
        if state in ['finality', 'proving', 'claiming', 'success', 'proof-failed', 'claim-failed', 'expired', 'release-window', 'release-ready']:
            assert browser('eval', '!document.querySelector(".payment-open") && ![...document.querySelectorAll("button")].some(b => b.textContent.includes("Pay with Zcash"))') == 'true'
            assert browser('eval', '!/Wallet fees are extra|Ironwood-capable|Pay by Zcash height|Could not copy/.test(document.body.innerText)') == 'true'
        if state in ['sold-out', 'halted', 'stalled', 'expired', 'proof-failed', 'claim-failed']:
            assert browser('eval', '!document.querySelector(".payment-open")') == 'true'
        for width, height, suffix in [(1440, 1100, ''), (390, 844, '-phone')]:
            browser('set', 'viewport', str(width), str(height))
            assert browser('eval', 'document.documentElement.scrollWidth <= innerWidth') == 'true'
            browser('screenshot', str(out / f'{state}{suffix}.png'), '--full')
        if state == 'expired':
            browser('click', 'button:has-text("Request release")')
            wait_for('document.body.innerText.includes("Release in")')
            assert browser('eval', '[...document.querySelectorAll("button")].find(b => b.textContent.trim() === "Release").disabled') == 'true'
            browser('screenshot', str(out / 'release-requested-phone.png'), '--full')
        if state == 'release-window':
            assert browser('eval', '[...document.querySelectorAll("button")].find(b => b.textContent.trim() === "Release").disabled') == 'true'
        if state == 'release-ready':
            browser('click', 'button:text-is("Release")')
            wait_for('!![...document.querySelectorAll("button")].find(b => b.textContent === "Reserve ticket")')
        assert not browser('errors'), f'Browser errors in {state}'
        print(f'PASS {state}: {args.url}/fixture.html?state={state}')
    print(f'PASS {len(states)} states, {len(states) * 2 + 1} screenshots, {time.monotonic() - started:.1f}s')
finally:
    browser('close')
