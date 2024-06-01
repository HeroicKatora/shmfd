## 0.2.3

Fix a bug where committing an entry would not commit the write offset to the
meta page, leading to confusing writes and sequencing after recovery.
