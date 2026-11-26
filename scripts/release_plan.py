"""Keep new signed releases and legacy immutable release plans distinct."""


def signing_mode(plan):
    if "signingMode" in plan:
        if plan["signingMode"] != "developer-id":
            raise ValueError("New release plans require Developer ID signing")
        return plan["signingMode"]
    # Historical plans must recover their original bytes and signing mode.
    return "developer-id" if plan["channel"] == "stable" else "ad-hoc"
