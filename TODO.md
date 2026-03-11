# Perk Search Index

- Build a dedicated perk-search index instead of scanning all project files on each query.
- Use the actual project JSON layout: perks live in `rows[].objects[]`.
- Extract and store the following perk fields in the index/database:
	- `project_id`
	- `project_name`
	- `row_id`
	- `row_title`
	- `object_id`
	- `title`
	- `description` from `text`
	- `points` from `scores[]` for display only, not full-text indexed, just first entry in array
	- `addons` from `addons[]`
	- optional `image_path`
- Confirm addon shape across projects and flatten it into a searchable/displayable text form.
- Treat points as structured display data, not searchable text.
- Keep image indexing optional.

# Image Handling For Perk View

- When opening the separate "all perks" view, prompt the user whether to include images.
- Show a warning that extracting images can take a lot of disk space and time.
- If the user chooses images:
	- extract images from project files into a separate folder
	- store extracted file paths in the perk database/index
	- avoid re-extracting unchanged project images
- If the user skips images:
	- build/use the perk index without image extraction
	- keep the search/index flow fast and small

# Index Lifecycle

- Store the perk index in an app-managed indexed SQLLite database in same directory as app.
- Track per-project index state so only changed projects are reindexed.
- Reindex when a project is added, removed, relinked, or its `project.json` changes.
- Support a full rebuild command in case the index gets out of sync.

# Search/UI Requirements

- Make the search fast enough for large libraries, including ~50,000 perks.
- Return lightweight results first; do not load all project JSON into the frontend.
- Add a separate perk browser/search view backed by the index.
- Show title, description, points, addons, project name, and optional image in results/details.
- Perk card should have title centered image on top (with image not cropped and taking it full width, so height should increase if it does not fit)
