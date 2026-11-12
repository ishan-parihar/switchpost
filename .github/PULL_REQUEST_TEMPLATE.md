## What this changes

<!-- One or two sentences: the behaviour that is different after this merges. -->

## Why

<!-- The problem this solves. A reviewer who reads only this section should understand
     the motivation without opening the diff. -->

## How it was verified

<!-- The check that proves it works, and how you ran it. -->

```
# the exact commands, and what they printed
```

## What a reviewer should look at first

<!-- The file and the line where the decision happens. -->

## Checklist

- [ ] I read `AGENTS.md` before changing anything.
- [ ] I did not delete a row from `TODO.md`; a row moves to done on evidence, and the
      verdict is appended.
- [ ] The change has a test, and the suite is green.
- [ ] I did not touch a credential path, a token store or a webhook signature check
      without saying so — those are security changes and need their own review.
- [ ] A change to a provider's behaviour says which provider, because the nine
      providers are not symmetric.
