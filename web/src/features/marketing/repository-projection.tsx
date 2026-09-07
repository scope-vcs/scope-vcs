import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'

export function RepositoryProjection(): ReactElement {
  return (
    <div className="demo">
      <figure
        aria-label="One repository with public and private files. The public clone contains shared folders. The examples folder is shared and then made private in a repeating illustration; internal code stays private."
        className="repository"
      >
        <div aria-hidden className="repository-header">
          <span className="repository-name">
            <GitBranch aria-hidden className="icon" />
            acme / toolkit
          </span>
          <span className="repository-branch">main</span>
        </div>
        <div aria-hidden className="repository-views">
          <div className="repository-column">
            <div className="repository-title">
              <LockKeyhole aria-hidden className="icon" />
              Your repository
            </div>
            <ul className="repository-files">
              <li className="repository-file">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">sdk/</span>
                <span className="visibility-label is-public">
                  <Globe aria-hidden className="icon" />
                  <span className="label-text">Public</span>
                </span>
              </li>
              <li className="repository-file">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">docs/</span>
                <span className="visibility-label is-public">
                  <Globe aria-hidden className="icon" />
                  <span className="label-text">Public</span>
                </span>
              </li>
              <li className="repository-file source-example">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">examples/</span>
                <span className="visibility-changing">
                  <span className="visibility-label is-private">
                    <LockKeyhole aria-hidden className="icon" />
                    <span className="label-text">Private</span>
                  </span>
                  <span className="visibility-label is-public">
                    <Globe aria-hidden className="icon" />
                    <span className="label-text">Public</span>
                  </span>
                </span>
              </li>
              <li className="repository-file">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">internal/</span>
                <span className="visibility-label is-private">
                  <LockKeyhole aria-hidden className="icon" />
                  <span className="label-text">Private</span>
                </span>
              </li>
              <li className="repository-file">
                <File aria-hidden className="icon" />
                <span className="repository-filename">README.md</span>
                <span className="visibility-label is-public">
                  <Globe aria-hidden className="icon" />
                  <span className="label-text">Public</span>
                </span>
              </li>
            </ul>
          </div>
          <div className="repository-column public-column">
            <div className="repository-title">
              <Globe aria-hidden className="icon" />
              Public clone
            </div>
            <ul className="repository-files">
              <li className="repository-file">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">sdk/</span>
                <Check aria-hidden className="icon" />
              </li>
              <li className="repository-file">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">docs/</span>
                <Check aria-hidden className="icon" />
              </li>
              <li className="repository-file shared-example">
                <Folder aria-hidden className="icon" />
                <span className="repository-filename">examples/</span>
                <Check aria-hidden className="icon" />
              </li>
              <li aria-hidden className="repository-file absent" />
              <li className="repository-file">
                <File aria-hidden className="icon" />
                <span className="repository-filename">README.md</span>
                <Check aria-hidden className="icon" />
              </li>
            </ul>
          </div>
        </div>
      </figure>
    </div>
  )
}
