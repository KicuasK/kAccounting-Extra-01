#!/usr/bin/env python3
"""Produce a disabled private-access planning policy; never invoke Tailscale.

This is deliberately NOT a `tailscale serve set-config` document. No proxy
backend is emitted until real macOS proxy compatibility has been accepted.
"""
import argparse
import json
import re


def dns_label(label):
    return bool(re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label))


def make_policy(hostname, login):
    if (len(hostname) > 253 or not hostname.endswith(".ts.net")
            or len(hostname.split(".")) < 4
            or not all(dns_label(label) for label in hostname.split("."))):
        raise ValueError("hostname must be explicit canonical lowercase device.tailnet.ts.net")
    if (len(login) > 254 or not login.isascii() or login.count("@") != 1):
        raise ValueError("login must be an explicit canonical ASCII Tailscale login")
    local, domain = login.split("@")
    if (not re.fullmatch(r"[A-Za-z0-9._+-]+", local) or ("." not in domain and domain != "github")
            or not all(dns_label(label) for label in domain.split("."))):
        raise ValueError("login must be an explicit canonical ASCII Tailscale login")
    return {
        "schema": "kAccounting-Extra-01.private-access-plan.v1",
        "deployment_enabled": False,
        "hostname": hostname,
        "origin": "https://" + hostname,
        "allowed_login": login,
        "regular_local_origin": "http://127.0.0.1:3000",
        "required_transport": "root-peer-unix-socket",
        "ingress_implementation_available": True,
        "actual_proxy_compatibility_verified": False,
        "blocked_reason": "Kernel-authenticated Unix ingress is implemented, but actual macOS Tailscale socket reachability, root peer and HTTPS acceptance remain unverified.",
        "funnel_allowed": False,
        "serve_config": None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hostname", required=True)
    parser.add_argument("--login", required=True)
    args = parser.parse_args()
    try:
        policy = make_policy(args.hostname, args.login)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps(policy, indent=2))


if __name__ == "__main__":
    main()
