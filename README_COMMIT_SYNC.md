Commit & Sync helper

Usage (PowerShell):

```powershell
pwsh ./scripts/commit-and-sync.ps1 -Message "Your commit message"
```

Usage (bash):

```bash
./scripts/commit-and-sync.sh "Your commit message"
```

If no commit message is provided the scripts will prompt; if there are no changes they will skip committing. The helpers will attempt to pull (using rebase + autostash) from the branch's upstream (or set upstream to `origin/<branch>` if possible) and then push the branch.

To add an `origin` remote or ensure git user config is present, run:

```powershell
pwsh ./scripts/setup-git.ps1 -OriginUrl "https://github.com/your/repo.git"
```
