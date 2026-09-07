import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'

export function ContributionFlow(): ReactElement {
  return (
    <figure
      aria-label="A request to handle empty API responses moves from a public clone to maintainer review. The maintainer reads the one-line change, comments that it looks good, then merges it into the full repository. Internal code remains private."
      className="contribution-flow"
    >
      <div aria-hidden className="request-progress">
        <div className="request-step">Submitted</div>
        <div className="request-step">Your review</div>
        <div className="request-step">Merged</div>
      </div>
      <div aria-hidden className="request-sheet">
        <div className="request-heading">
          <GitBranch className="icon" />
          <h3>Handle empty responses</h3>
          <div className="request-state-stack">
            <span className="state-submitted">Open</span>
            <span className="state-review">In review</span>
            <span className="state-merged">Merged</span>
          </div>
        </div>
        <div className="request-scenes">
          <div className="request-scene submission-scene">
            <div className="scene-context">
              <Globe className="icon" />
              Public clone
            </div>
            <ul className="scene-tree">
              <li className="scene-file submitted-file">
                <File className="icon" />
                sdk/api.ts
                <span className="file-state">+1</span>
              </li>
              <li className="scene-file">
                <Folder className="icon" />
                docs/
              </li>
              <li className="scene-file">
                <File className="icon" />
                README.md
              </li>
            </ul>
            <div className="scene-receipt">
              <Check className="icon" />
              Submitted to your review queue
            </div>
          </div>
          <div className="request-scene review-scene">
            <div className="review-file-header">
              <span>
                <File className="icon" />
                sdk/api.ts
              </span>
              <span>+1 line</span>
            </div>
            <div className="review-code">
              <span>  const response = await fetch(url);</span>
              <span className="added-line">
                + if (response.status === 204) return null;
              </span>
              <span>  return response.json();</span>
            </div>
            <div className="maintainer-review">
              <span className="reviewer-mark">M</span>
              <div>
                <div className="reviewer-name">Maintainer</div>
                <p className="review-comment-text">
                  Empty responses return null. Looks good.
                </p>
              </div>
            </div>
            <div className="review-decision">
              <Check className="icon" />
              Ready to merge
            </div>
          </div>
          <div className="request-scene merged-scene">
            <div className="scene-context">
              <GitBranch className="icon" />
              Your repository
              <span className="scene-branch">main</span>
            </div>
            <ul className="scene-tree">
              <li className="scene-file merged-file">
                <File className="icon" />
                sdk/api.ts
                <span className="file-state">
                  <Check className="icon" />
                  Updated
                </span>
              </li>
              <li className="scene-file">
                <Folder className="icon" />
                docs/
              </li>
              <li className="scene-file">
                <Folder className="icon" />
                internal/
                <span className="file-state private-state">
                  <LockKeyhole className="icon" />
                  Private
                </span>
              </li>
              <li className="scene-file">
                <File className="icon" />
                README.md
              </li>
            </ul>
            <div className="scene-receipt">
              <Check className="icon" />
              Merged by the maintainer
            </div>
          </div>
        </div>
      </div>
    </figure>
  )
}
