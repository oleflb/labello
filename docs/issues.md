# Working with issues

Every check box item is an issue. An issues that is checked is finished and should be skipped.
When working on an issue, use the following workflow:

1. use analyze the issue
2. reproduce it, if it is visual look at it in the inspector with mcp
3. if it is a valid issue create a new branch from the current one
4. plan a fix
5. implement the fix
6. validate
7. Review
8. then commit

Only after that continue with the next issue.

# Feedback Issues

- [x] Reorder the per-task columns in the stats view from left to right to match the workflow timeline: Annotate, Review, Approve.
- [x] Hide **Create a dataset** in Setup from users who do not have permission to create datasets.
  - Not quite if this is actually still an issue.
- [ ] Allow users to return to the previous skipped or submitted assignment to correct accidental skips or submissions.
- [ ] Almost all text boxes are not fit to the size of the font of the actual text in the text box.
  - One example that looks good already is the API URL box in Setup.
  - Also boxes where large amounts of text need to be fit into like descriptions should be resizable text boxes.
  - All single line text boxes should be vertically centered. The height of the text box should only be adjusted to fit the text if the height does not need to match another element in its proximity. E.g. the text boxes in Admin sections actually look good as high as they are, since they match the height of some buttons next to them. 
- [ ] The navigation dropdown is very awkward in views with the image view.
  - I think we can remove the upper layer of that menu hierarchy and simply put all elements of that menu in the bar.
  - On mobile/small narrow screen, it is still necessary. The sizing of the menu items needs to be improved. The status item from that menu can be removed entirely.
- [ ] The admin view has some layouting issues:
  - In People, the role checkboxes are offset and do not fit. Also the Person column should be centered vertically.
  - All background cards in the sections should be full width. 
- [ ] The normal non-highlighted button does not look enough like a button. Make it just slightly more different from the background and other text.
