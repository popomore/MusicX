"""Merge authenticated feeds, retaining the newest three updates per channel."""
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

from appcast import SPARKLE, read_feed, version_key


def merge(current, previous, output):
    feed = read_feed(current)
    channel = feed.find("channel")
    new_items = channel.findall("item")
    if len(new_items) != 1:
        raise ValueError("Expected one newly packaged update")
    new = new_items[0]
    old = read_feed(previous).findall("./channel/item") if previous else []
    for item in old:
        if version_key(item) > version_key(new):
            raise ValueError("Build number would move the update feed backwards")
        if version_key(item) == version_key(new) and ET.tostring(item) != ET.tostring(new):
            raise ValueError("Build number already identifies another release")
    for item in new_items:
        channel.remove(item)
    retained, counts = set(), {}
    for item in sorted(new_items + old, key=version_key, reverse=True):
        branch = item.findtext(SPARKLE + "channel")
        if branch not in (None, "preview"):
            raise ValueError("Unknown update channel")
        identity = (branch, version_key(item))
        if identity in retained or counts.get(branch, 0) >= 3:
            continue
        retained.add(identity)
        counts[branch] = counts.get(branch, 0) + 1
        channel.append(item)
    ET.ElementTree(feed).write(output, encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    merge(Path(sys.argv[1]), None if sys.argv[2] == "-" else Path(sys.argv[2]), Path(sys.argv[3]))
