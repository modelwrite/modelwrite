# Tasks

Numbered, in order, with the exact clicks. Every step below was checked against the UI code
in server/src/ui/ and, where it is read-only, against the running trial. Where a step is not
yet possible in the workbench, the step says so instead of inventing a button.

The workbench is plain server-rendered HTML with no build step. Every link and form field
below is the name you will see on the page. In the trial the service runs in open mode, so
you are anonymous admin and every form is offered; on a configured deployment a form you
cannot submit is not shown at all, and the server still refuses the request if you send it
directly.

## 1. Create a project

1. Open the trial (or your instance). The bare host redirects to the project list.
2. At the bottom of the list is the "New project" form. Type a name and click "Create
   project".
3. You land on the new project's Changes page. Creating a project needs admin permission;
   the form is offered only to a caller who may administer.

## 2. Start a model from empty, or paste one

1. On the empty project's Changes page, click "Start a model".
2. The Overview shows "This project has no model yet, start one or import a legacy model"
   and two forms.
3. Start from empty: click "Start from empty". The commit message is pre-filled as "start
   the model". This stores a minimal valid OKF document: zero structure, zero requirements,
   one graph node and a state machine, which is what the validator demands. An empty model
   cannot be literally empty.
4. Paste a document: type a commit message, paste OKF JSON, click "Create from pasted
   document". The pasted document is validated before anything is stored. Invalid JSON or an
   invalid document re-renders the form with the validator's errors and stores nothing.
5. Either way you land on the model at the new commit.

## 3. Add blocks and requirements

1. From Overview or Structure, click "Add element". The link is offered only to a writer.
2. The form has: Kind (block or requirement), id, name, reqId (requirements only), text
   (requirements only), and a commit message.
3. A requirement needs a non-empty reqId. A block does not. A duplicate or empty id is
   refused with the validator's error, next to the form, and nothing is stored.
4. Click "Add element". You land on the model at the new commit. A block lands in the
   structure list; a requirement lands in the requirements table; both get a matching graph
   node.

## 4. Import a legacy model and read the loss report

1. Open the Import section in the left rail.
2. Choose the binding. Today the only binding is sysml-v1-xmi@2.4, which reads a stated
   subset: blocks, requirements, properties, and Satisfy/Allocate traceability.
3. Fill the branch, the commit message, and paste the source artifact (raw text or base64)
   into the "Source artifact" box. Click "Import".
4. If the import is lossy, you get "Import refused: blocking losses". Nothing was committed.
   The page states that the source artifact was retained byte for byte and content-addressed.
5. The loss report groups every mapping as unmappable, lossy, or exact, each naming its
   subject. To commit, check every blocking loss by name and click "Accept the checked
   losses and import". A loss left unchecked refuses the import again.
6. Read the fidelity note on the same page before you accept. It states the honest
   boundary: the engine measured the binding's own OKF to XMI to OKF round trip; the native
   XMI to OKF read of your artifact is not independently measured. The loss report is the
   binding's own account of that read.

A lossy migration must be accepted by name. There is no way to commit one silently.

## 5. Make a version, edit it, compare it, make it current

Make a version:

1. In the context bar, click "New version". It is offered only to a writer.
2. Name the version and choose the commit to branch from. Click "Create version". A version
   is a branch, so this posts to the branch endpoint and lands you on the new branch's
   Overview.

Edit it:

1. In Structure, click "edit" beside an element.
2. Change id, name, documentation, stereotypes (comma-separated), or attributes. Type a
   commit message. Click "Commit".
3. The editor takes a short lease on the element for the request and refuses if another
   holder has it, naming the holder. Renaming an element carries its references so the graph
   does not go stale.

Compare it:

1. In the context bar, click "Compare", or use the Compare form on the Changes page.
2. Type or choose the from and to endpoints (a branch or a commit). Click "Compare".
3. You see the engine's section-scoped diff and the gate verdict.
4. This comparison is not recorded. The page says so, and points you at the gate endpoint
   to persist a run. See task 7.

Make it current:

1. View a non-main branch. The context bar then offers "Make current".
2. The page runs the gate first and shows the verdict, then offers "Merge and make current".
   It merges the branch into the main line. The gate verdict here is not recorded either.

## 6. Ask the assist to draft a change and accept it

1. Open the Assist section, or the Assist panel on Overview. Both are offered only to a
   writer.
2. The panel states the reasoner status. With a live reasoner configured it says which; with
   none it says so and shows no request box.
3. Type the change in words and click "Submit".
4. You land on the Assist review page: the proposed changes, each with its action, rationale
   and confidence (low-confidence ones are marked), plus any gaps the reasoner named, and an
   Accept form.
5. Review the changes. Type a commit message and click "Accept".
6. You land on "Proposal accepted": the commit, and a provenance block naming the proposing
   agent and the accepting human.

Nothing commits until you click Accept. The panel records a proposal and shows it; it never
writes a model on its own. Acceptance needs write permission, which is the human's, and the
commit path re-checks the acceptance inside its own transaction.

## 7. Run the gate and read the evidence

There is no workbench button that records a gate run. The gate records a run only through
the API, and runs automatically at import.

1. Run it: POST /projects/<project>/gate with a JSON body. Both fields are commit hashes,
   not branch names. It needs write permission.

   curl -X POST http://127.0.0.1:8080/projects/coffee-machine/gate \
     -H "content-type: application/json" \
     -d '{"reference":"<commit-hash>","candidate":"<commit-hash>"}'

   Copy the full hash from the version selector or the branch list. On the trial, the
   coffee-machine main tip is d427e847bf4d875d06fc7f2a5b47bb0a3dc53855a61850195c4ec78e7d45fc36
   today; your instance will differ.

2. Read it: open the Checks section in the left rail. It lists recorded gate runs, newest
   first, passed or failed, with both hashes and the evidence file name. Click a run for the
   detail: fidelity, integration, coverage, validation, and the full evidence record.
3. A failed gate is a successful outcome of running a gate. It renders as a page, never as
   an error.

The make-current and compare pages also compute a gate verdict, but they record nothing. The
only recorded runs are the ones this endpoint and the import write.

## 8. Find what has been run on a revision

1. The Checks section lists runs across the whole project.
2. To ask about one commit specifically, use the API:

   curl -s http://127.0.0.1:8080/projects/coffee-machine/commits/<commit-hash>/checks

3. The answer carries a checked flag. false means UNCHECKED: the commit exists and nobody
   has checked it. The flag exists so an empty list can never read like a pass. It needs
   write or review permission, exactly like the project-wide list.
